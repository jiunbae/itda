//! Home window: the address book, settings and connection flow, rendered by WebView2
//! from the embedded `web/` assets. Each connection runs in its own `itda --session`
//! process; this process tracks them through their stdout events.

use crate::session::Launch;
use crate::store::{self, Host, Settings};
use crate::ui::wide;
use serde_json::{json, Value};
use std::borrow::Cow;
use std::cell::RefCell;
use std::collections::HashMap;
use std::io::{BufRead, BufReader, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::num::NonZeroIsize;
use std::os::windows::process::CommandExt;
use std::process::{Child, Command, Stdio};
use std::ptr::{null, null_mut};
use std::sync::atomic::{AtomicIsize, Ordering::Relaxed};
use std::sync::Mutex;
use std::time::Duration;
use windows_sys::Win32::Foundation::*;
use windows_sys::Win32::Graphics::Dwm::DwmSetWindowAttribute;
use windows_sys::Win32::Graphics::Gdi::*;
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::System::Registry::{RegGetValueW, HKEY_CURRENT_USER, RRF_RT_REG_DWORD};
use windows_sys::Win32::System::Threading::CreateMutexW;
use windows_sys::Win32::UI::HiDpi::GetDpiForWindow;
use windows_sys::Win32::UI::Shell::*;
use windows_sys::Win32::UI::WindowsAndMessaging::*;
use wry::raw_window_handle::{HandleError, HasWindowHandle, RawWindowHandle, Win32WindowHandle, WindowHandle};
use wry::{http, Rect, WebView, WebViewBuilder, WebViewBuilderExtWindows};

const WM_EVENT: u32 = WM_APP + 20;
const WM_TRAY: u32 = WM_APP + 21;
const TRAY_OPEN: usize = 1;
const TRAY_QUIT: usize = 2;
const TRAY_SHARE: usize = 3;
const TRAY_HOST0: usize = 100;
const CREATE_NO_WINDOW: u32 = 0x0800_0000;
const DWMWA_USE_IMMERSIVE_DARK_MODE: u32 = 20;
const VERSION: &str = env!("CARGO_PKG_VERSION");

// Embedded UI. Set ITDA_WEB_DIR to serve them from disk while working on the UI.
const INDEX_HTML: &str = include_str!("../web/index.html");
const APP_CSS: &str = include_str!("../web/app.css");
const APP_JS: &str = include_str!("../web/app.js");
const ICON_SVG: &str = include_str!("../assets/itda.svg");
const HELP_JS: &str = include_str!("../web/help.js");
const HELP_CSS: &str = include_str!("../web/help.css");
/// Help-center screenshots (WebP), embedded.
const HELP_IMG: &[(&str, &[u8])] = &[
    ("itda-home", include_bytes!("../web/help/itda-home.webp")),
    ("itda-nearby", include_bytes!("../web/help/itda-nearby.webp")),
    ("itda-accounts", include_bytes!("../web/help/itda-accounts.webp")),
    ("itda-account-edit", include_bytes!("../web/help/itda-account-edit.webp")),
    ("itda-edit", include_bytes!("../web/help/itda-edit.webp")),
    ("itda-share", include_bytes!("../web/help/itda-share.webp")),
    ("itda-web", include_bytes!("../web/help/itda-web.webp")),
    ("phone-login", include_bytes!("../web/help/phone-login.webp")),
    ("phone-list", include_bytes!("../web/help/phone-list.webp")),
    ("phone-edit", include_bytes!("../web/help/phone-edit.webp")),
    ("phone-view", include_bytes!("../web/help/phone-view.webp")),
    ("phone-keys", include_bytes!("../web/help/phone-keys.webp")),
];

/// Messages from worker threads, drained on the UI thread.
enum Event {
    /// A line from a session process.
    Session(String, Value),
    /// A session process exited.
    Exit(String),
    Online(HashMap<String, bool>),
    Found(Vec<crate::mdns::Found>),
    Tailnet(crate::tailscale::Status, bool),
    /// Result of turning `tailscale serve` on/off.
    TsServe(Result<(), String>),
    /// Someone started or stopped viewing this PC.
    Share,
    /// An address-book change from the web viewer (a page command); answered when saved.
    WebEdit(Value, std::sync::mpsc::Sender<()>),
}

static QUEUE: Mutex<Vec<Event>> = Mutex::new(Vec::new());
static HWND_HOME: AtomicIsize = AtomicIsize::new(0);

fn post(e: Event) {
    QUEUE.lock().unwrap().push(e);
    unsafe { PostMessageW(HWND_HOME.load(Relaxed) as HWND, WM_EVENT, 0, 0) };
}

struct Sess {
    child: Child,
    hwnd: isize,
    /// "connecting" | "connected" | "reconnecting"
    state: String,
    stage: String,
    /// Password to store once the connection is proven to work.
    save_pw: Option<String>,
}

#[derive(Default)]
struct Home {
    hwnd: HWND,
    hosts: Vec<Host>,
    accounts: Vec<store::Account>,
    settings: Settings,
    sessions: HashMap<String, Sess>,
    online: HashMap<String, bool>,
    found: Vec<crate::mdns::Found>,
    /// Address passed on the command line: offer to connect once the page is up.
    pending_quick: Option<String>,
    ready: bool,
}

thread_local! {
    static HOME: RefCell<Home> = RefCell::new(Home::default());
    /// Kept apart from HOME: WebView2 calls can re-enter the window procedure, so the
    /// webview is only ever borrowed shared (except when created and dropped).
    static WEBVIEW: RefCell<Option<WebView>> = const { RefCell::new(None) };
}
static TRAY_MSG: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);

fn home<R>(f: impl FnOnce(&mut Home) -> R) -> R {
    HOME.with(|h| f(&mut h.borrow_mut()))
}

struct Win(HWND);

impl HasWindowHandle for Win {
    fn window_handle(&self) -> Result<WindowHandle<'_>, HandleError> {
        let raw = Win32WindowHandle::new(NonZeroIsize::new(self.0 as isize).ok_or(HandleError::Unavailable)?);
        Ok(unsafe { WindowHandle::borrow_raw(RawWindowHandle::Win32(raw)) })
    }
}

fn system_dark() -> bool {
    let mut v: u32 = 1;
    let mut n = 4u32;
    let key = wide(r"Software\Microsoft\Windows\CurrentVersion\Themes\Personalize");
    let val = wide("AppsUseLightTheme");
    unsafe { RegGetValueW(HKEY_CURRENT_USER, key.as_ptr(), val.as_ptr(), RRF_RT_REG_DWORD, null_mut(), &mut v as *mut u32 as _, &mut n) };
    v == 0
}

fn is_dark(s: &Settings) -> bool {
    match s.theme.as_str() {
        "dark" => true,
        "light" => false,
        _ => system_dark(),
    }
}

unsafe fn apply_theme(h: HWND) {
    let dark: BOOL = home(|m| is_dark(&m.settings)) as BOOL;
    DwmSetWindowAttribute(h, DWMWA_USE_IMMERSIVE_DARK_MODE, &dark as *const BOOL as _, 4);
}

pub fn run(quick: Option<String>) {
    unsafe {
        // One home window: a second launch just brings the first to the front.
        // Test runs are single-instance among themselves, apart from a real one.
        let (name, title) = match std::env::var("ITDA_TEST_HTTP") {
            Ok(port) => (format!("Local\\itda-home-test-{port}"), format!("{} (test {port})", crate::APP)),
            Err(_) => ("Local\\itda-home".to_string(), crate::APP.to_string()),
        };
        CreateMutexW(null(), 0, wide(&name).as_ptr());
        if GetLastError() == ERROR_ALREADY_EXISTS {
            let w = FindWindowW(wide("itda_home").as_ptr(), wide(&title).as_ptr());
            if !w.is_null() {
                if let Some(q) = &quick {
                    // Hand the address to the running home.
                    let b = q.as_bytes();
                    let cd = windows_sys::Win32::System::DataExchange::COPYDATASTRUCT { dwData: 1, cbData: b.len() as u32, lpData: b.as_ptr() as _ };
                    SendMessageW(w, WM_COPYDATA, 0, &cd as *const _ as isize);
                }
                ShowWindow(w, SW_SHOW);
                ShowWindow(w, SW_RESTORE);
                SetForegroundWindow(w);
            }
            return;
        }
        let settings = store::load_settings();
        let hosts = store::load_hosts();
        let accounts = store::load_accounts();
        let dark = is_dark(&settings);
        home(|m| {
            m.settings = settings;
            m.hosts = hosts;
            m.accounts = accounts;
            m.pending_quick = quick;
        });

        let hinst = GetModuleHandleW(null());
        let icon = LoadIconW(hinst, 1 as _);
        let cls = wide("itda_home");
        let wc = WNDCLASSEXW {
            cbSize: size_of::<WNDCLASSEXW>() as u32,
            lpfnWndProc: Some(wndproc),
            hInstance: hinst,
            hIcon: icon,
            hIconSm: icon,
            hCursor: LoadCursorW(null_mut(), IDC_ARROW),
            hbrBackground: CreateSolidBrush(if dark { 0x0020_2020 } else { 0x00f3_f3f3 }),
            lpszClassName: cls.as_ptr(),
            ..std::mem::zeroed()
        };
        RegisterClassExW(&wc);
        let h = CreateWindowExW(0, cls.as_ptr(), wide(&title).as_ptr(), WS_OVERLAPPEDWINDOW, CW_USEDEFAULT, 0, 100, 100,
            null_mut(), null_mut(), hinst, null());
        HWND_HOME.store(h as isize, Relaxed);
        home(|m| m.hwnd = h);
        apply_theme(h);
        let dpi = GetDpiForWindow(h) as i32;
        let (ww, wh) = (1080 * dpi / 96, 720 * dpi / 96);
        let mut wa: RECT = std::mem::zeroed();
        SystemParametersInfoW(SPI_GETWORKAREA, 0, &mut wa as *mut _ as _, 0);
        let (ww, wh) = (ww.min(wa.right - wa.left), wh.min(wa.bottom - wa.top));
        SetWindowPos(h, null_mut(), (wa.left + wa.right - ww) / 2, (wa.top + wa.bottom - wh) / 2, ww, wh, SWP_NOZORDER);

        let bridge = std::env::var("ITDA_TEST_HTTP").ok().and_then(|p| p.parse::<u16>().ok());
        match bridge.map(|p| Ok(test_bridge::start(p))).unwrap_or_else(|| build_webview(h, dark).map(|wv| WEBVIEW.with(|w| *w.borrow_mut() = Some(wv)))) {
            Ok(()) => apply_zoom(home(|m| m.settings.zoom)),
            Err(e) => {
                crate::log!("webview: {e}");
                crate::ui::message(
                    &format!("화면 구성 요소(Microsoft Edge WebView2)를 시작할 수 없습니다.\n\n{e}\n\nhttps://go.microsoft.com/fwlink/p/?LinkId=2124703 에서 설치할 수 있습니다."),
                    true,
                );
                return;
            }
        }
        resize_webview(h);
        tray_add(h);
        TRAY_MSG.store(RegisterWindowMessageW(wide("TaskbarCreated").as_ptr()), Relaxed);
        ShowWindow(h, SW_SHOW);
        SetForegroundWindow(h);
        start_probes();
        let w = home(|m| m.settings.web.clone());
        if w.on {
            apply_web(&w);
        }
        let sh = home(|m| m.settings.share.clone());
        if sh.on {
            apply_share(&sh);
        }
        scan();

        let mut msg: MSG = std::mem::zeroed();
        while GetMessageW(&mut msg, null_mut(), 0, 0) > 0 {
            TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
        tray_remove(h);
    }
}

fn asset(path: &str) -> Option<(Cow<'static, [u8]>, &'static str)> {
    let path = path.trim_start_matches('/');
    let path = if path.is_empty() { "index.html" } else { path };
    if let Some(id) = path.strip_prefix("thumb/").and_then(|p| p.strip_suffix(".png")) {
        if id.chars().all(|c| c.is_ascii_hexdigit()) {
            return std::fs::read(store::thumb_path(id)).ok().map(|b| (Cow::Owned(b), "image/png"));
        }
        return None;
    }
    if let Some(name) = path.strip_prefix("help/").and_then(|p| p.strip_suffix(".webp")) {
        return HELP_IMG.iter().find(|(n, _)| *n == name).map(|(_, b)| (Cow::Borrowed(*b), "image/webp"));
    }
    let mime = match path.rsplit('.').next() {
        Some("html") => "text/html; charset=utf-8",
        Some("css") => "text/css; charset=utf-8",
        Some("js") => "text/javascript; charset=utf-8",
        Some("svg") => "image/svg+xml",
        Some("png") => "image/png",
        _ => "application/octet-stream",
    };
    if let Some(dir) = std::env::var_os("ITDA_WEB_DIR") {
        if !path.contains("..") {
            return std::fs::read(std::path::Path::new(&dir).join(path)).ok().map(|b| (Cow::Owned(b), mime));
        }
    }
    let body: &'static str = match path {
        "index.html" => INDEX_HTML,
        "app.css" => APP_CSS,
        "app.js" => APP_JS,
        "icon.svg" => ICON_SVG,
        "help.js" => HELP_JS,
        "help.css" => HELP_CSS,
        _ => return None,
    };
    Some((Cow::Borrowed(body.as_bytes()), mime))
}

fn build_webview(h: HWND, dark: bool) -> wry::Result<WebView> {
    let data = std::env::var_os("LOCALAPPDATA").map(std::path::PathBuf::from).unwrap_or_else(std::env::temp_dir).join(crate::APP).join("WebView2");
    let mut ctx = wry::WebContext::new(Some(data));
    let bg = if dark { (32, 32, 32, 255) } else { (243, 243, 243, 255) };
    let devtools = std::env::var_os("ITDA_DEVTOOLS").is_some();
    // Test hook: expose the DevTools protocol so the UI can be driven and captured.
    let mut args = "--disable-features=msWebOOUI,msPdfOOUI,msSmartScreenProtection".to_string();
    if let Ok(port) = std::env::var("ITDA_DEBUG_PORT") {
        args.push_str(&format!(" --remote-debugging-port={port}"));
    }
    WebViewBuilder::new_with_web_context(&mut ctx)
        .with_additional_browser_args(args)
        .with_custom_protocol("itda".into(), |_id, req| {
            let resp = match asset(req.uri().path()) {
                Some((body, mime)) => http::Response::builder().header("Content-Type", mime).header("Cache-Control", "no-store").body(body),
                None => http::Response::builder().status(404).body(Cow::Borrowed(&b"not found"[..])),
            };
            resp.unwrap_or_else(|_| http::Response::new(Cow::Borrowed(&b""[..])))
        })
        .with_ipc_handler(|req| {
            let body = req.into_body();
            match serde_json::from_str::<Value>(&body) {
                Ok(v) => {
                    // Handle after this callback returns: WebView2 must not be re-entered from here.
                    QUEUE.lock().unwrap().push(Event::Session("__ipc".into(), v));
                    unsafe { PostMessageW(HWND_HOME.load(Relaxed) as HWND, WM_EVENT, 0, 0) };
                }
                Err(e) => crate::log!("ipc: bad message {e}"),
            }
        })
        .with_navigation_handler(|url| {
            // External links open in the browser, never inside the app.
            if url.starts_with("http://itda.localhost/") || url.starts_with("itda://") {
                return true;
            }
            if url.starts_with("https://") {
                let u = wide(&url);
                unsafe { ShellExecuteW(null_mut(), wide("open").as_ptr(), u.as_ptr(), null(), null(), SW_SHOWNORMAL) };
            }
            false
        })
        .with_new_window_req_handler(|_url, _features| wry::NewWindowResponse::Deny)
        .with_url("itda://localhost/index.html")
        .with_background_color(bg)
        .with_devtools(devtools)
        .with_hotkeys_zoom(false)
        .with_browser_accelerator_keys(false)
        .with_default_context_menus(devtools)
        .build_as_child(&Win(h))
}

/// The home's own zoom (percent). WebView2's zoom shrinks the CSS viewport, so the page's
/// narrow-window layout takes over at large sizes.
fn apply_zoom(pct: u16) {
    let z = if (50..=300).contains(&pct) { pct } else { 100 };
    WEBVIEW.with(|w| {
        if let Some(wv) = w.borrow().as_ref() {
            let _ = wv.zoom(z as f64 / 100.0);
        }
    });
}

unsafe fn resize_webview(h: HWND) {
    let mut rc: RECT = std::mem::zeroed();
    GetClientRect(h, &mut rc);
    WEBVIEW.with(|w| {
        if let Some(wv) = w.borrow().as_ref() {
            let _ = wv.set_bounds(Rect {
                position: wry::dpi::PhysicalPosition::new(0, 0).into(),
                size: wry::dpi::PhysicalSize::new(rc.right as u32, rc.bottom as u32).into(),
            });
        }
    });
}

/// Send a message to the page.
fn send(v: Value) {
    let js = format!("window.itda&&itda.recv({v})");
    if !home(|m| m.ready) {
        return;
    }
    if test_bridge::active() {
        return test_bridge::push(v.to_string());
    }
    WEBVIEW.with(|w| {
        if let Some(wv) = w.borrow().as_ref() {
            let _ = wv.evaluate_script(&js);
        }
    });
}

fn thumb_version(id: &str) -> u64 {
    std::fs::metadata(store::thumb_path(id))
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map_or(0, |d| d.as_millis() as u64)
}

fn host_json(h: &Host, accounts: &[store::Account]) -> Value {
    let mut v = serde_json::to_value(h).unwrap_or(Value::Null);
    let login = store::resolve(h, accounts);
    v["has_pw"] = json!(login.pass.is_some());
    v["login_user"] = json!(login.user);
    v["thumb"] = json!(thumb_version(&h.id));
    v
}

fn sessions_json(m: &Home) -> Value {
    Value::Object(m.sessions.iter().map(|(id, s)| (id.clone(), json!({"state": s.state, "stage": s.stage}))).collect())
}

fn accounts_json(m: &Home) -> Value {
    json!(m
        .accounts
        .iter()
        .map(|a| {
            let mut v = serde_json::to_value(a).unwrap_or(Value::Null);
            v["has_pw"] = json!(store::get_password(&store::acct_key(&a.id)).is_some());
            v["has_keypass"] = json!(store::get_password(&store::acct_keypass(&a.id)).is_some());
            v["uses"] = json!(m.hosts.iter().filter(|h| h.account == a.id).count());
            v
        })
        .collect::<Vec<_>>())
}

fn save_accounts() {
    home(|m| store::save_accounts(&m.accounts));
}

fn send_hosts() {
    let v = home(|m| json!({"type": "hosts", "hosts": m.hosts.iter().map(|h| host_json(h, &m.accounts)).collect::<Vec<_>>(), "accounts": accounts_json(m)}));
    send(v);
}

fn send_state() {
    let v = home(|m| {
        json!({
            "type": "state",
            "version": VERSION,
            "hosts": m.hosts.iter().map(|h| host_json(h, &m.accounts)).collect::<Vec<_>>(),
            "accounts": accounts_json(m),
            "settings": m.settings,
            "sessions": sessions_json(m),
            "online": m.online,
            "found": m.found,
            "dark": system_dark(),
            "ssh_ok": crate::tunnel::ssh_exe().is_some(),
            "tailnet": *TAILNET.lock().unwrap(),
            "quick": m.pending_quick.take(),
        })
    });
    send(v);
}

fn send_session(id: &str) {
    let v = home(|m| match m.sessions.get(id) {
        Some(s) => json!({"type": "session", "id": id, "state": s.state, "stage": s.stage}),
        None => json!({"type": "session", "id": id, "state": "idle"}),
    });
    send(v);
}

fn save_hosts() {
    home(|m| store::save_hosts(&m.hosts));
}

// ---------------------------------------------------------------- page commands

fn s(v: &Value, k: &str) -> String {
    v.get(k).and_then(Value::as_str).unwrap_or("").to_string()
}

unsafe fn on_ipc(v: Value) {
    let id = s(&v, "id");
    match s(&v, "cmd").as_str() {
        "ready" => {
            home(|m| m.ready = true);
            send_state();
        }
        "connect" => {
            let pass = v.get("password").and_then(Value::as_str).map(str::to_string);
            let save = v.get("save").and_then(Value::as_bool).unwrap_or(false);
            connect(&id, pass, save);
        }
        "cancel" | "disconnect" => stop_session(&id),
        "focus" => focus_session(&id),
        "save_host" => {
            let Ok(mut host) = serde_json::from_value::<Host>(v["host"].clone()) else { return };
            host.host = host.host.trim().to_string();
            host.name = host.name.trim().to_string();
            host.user = host.user.trim().to_string();
            if host.id.is_empty() {
                host.id = store::new_id();
            }
            match v.get("password").and_then(Value::as_str) {
                Some(p) if v["save_password"].as_bool() == Some(true) && !p.is_empty() => store::set_password(&host.id, Some(p)),
                _ if v["save_password"].as_bool() == Some(false) => store::set_password(&host.id, None),
                _ => {}
            }
            let id = host.id.clone();
            home(|m| match m.hosts.iter_mut().find(|x| x.id == id) {
                Some(x) => {
                    // Keep fields the editor doesn't own.
                    host.last_used = x.last_used;
                    host.remote_name = std::mem::take(&mut x.remote_name);
                    host.remote_size = std::mem::take(&mut x.remote_size);
                    *x = host;
                }
                None => m.hosts.push(host),
            });
            save_hosts();
            send_hosts();
            send(json!({"type": "saved", "id": id, "then": v.get("then")}));
            probe_now();
        }
        "delete_host" => {
            stop_session(&id);
            home(|m| m.hosts.retain(|x| x.id != id));
            store::set_password(&id, None);
            let _ = std::fs::remove_file(store::thumb_path(&id));
            save_hosts();
            send_hosts();
        }
        "duplicate" => {
            let copy = home(|m| m.hosts.iter().find(|x| x.id == id).cloned());
            if let Some(mut c) = copy {
                c.id = store::new_id();
                c.name = format!("{} (복사본)", c.label());
                c.last_used = 0;
                c.favorite = false;
                home(|m| m.hosts.push(c));
                save_hosts();
                send_hosts();
            }
        }
        "favorite" => {
            let on = v["on"].as_bool().unwrap_or(false);
            home(|m| m.hosts.iter_mut().filter(|x| x.id == id).for_each(|x| x.favorite = on));
            save_hosts();
            send_hosts();
        }
        "trust_new_key" => {
            // The person confirmed the Mac's SSH key legitimately changed.
            if let Some(h) = home(|m| m.hosts.iter().find(|x| x.id == id).cloned()) {
                let (host, _) = crate::tunnel::split_host(&h.host, 5900);
                crate::tunnel::forget_host_key(&host, if h.ssh.port == 0 { 22 } else { h.ssh.port });
                connect(&id, None, false);
            }
        }
        "ssh_available" => send(json!({"type": "ssh_available", "ok": crate::tunnel::ssh_exe().is_some()})),
        "save_account" => {
            let Ok(mut a) = serde_json::from_value::<store::Account>(v["account"].clone()) else { return };
            a.user = a.user.trim().to_string();
            a.name = a.name.trim().to_string();
            if a.id.is_empty() {
                a.id = store::new_id();
            }
            let secret = |field: &str, key: String| match v.get(field).and_then(Value::as_str) {
                Some("") | None => {}
                Some(p) => store::set_password(&key, Some(p)),
            };
            secret("password", store::acct_key(&a.id));
            secret("key_pass", store::acct_keypass(&a.id));
            if v["clear_key_pass"].as_bool() == Some(true) {
                store::set_password(&store::acct_keypass(&a.id), None);
            }
            let id = a.id.clone();
            home(|m| match m.accounts.iter_mut().find(|x| x.id == id) {
                Some(x) => *x = a,
                None => m.accounts.push(a),
            });
            // Optionally attach the connection being edited.
            if let Some(hid) = v.get("attach").and_then(Value::as_str) {
                home(|m| m.hosts.iter_mut().filter(|h| h.id == hid).for_each(|h| h.account = id.clone()));
                save_hosts();
            }
            save_accounts();
            send_hosts();
            send(json!({"type": "account_saved", "id": id}));
        }
        "delete_account" => {
            let gone = home(|m| {
                let a = m.accounts.iter().find(|x| x.id == id).cloned();
                m.accounts.retain(|x| x.id != id);
                // Connections keep the user name and ask for a password next time.
                for h in m.hosts.iter_mut().filter(|h| h.account == id) {
                    h.account.clear();
                    if let Some(a) = &a {
                        h.user = a.user.clone();
                    }
                }
                a
            });
            if gone.is_some() {
                store::set_password(&store::acct_key(&id), None);
                store::set_password(&store::acct_keypass(&id), None);
                save_accounts();
                save_hosts();
                send_hosts();
            }
        }
        "group_accounts" => {
            // Hosts that sign in with the same name become one account.
            let user = s(&v, "user");
            let members: Vec<String> = home(|m| m.hosts.iter().filter(|h| h.account.is_empty() && h.user == user).map(|h| h.id.clone()).collect());
            if user.is_empty() || members.is_empty() {
                return;
            }
            let a = store::Account { id: store::new_id(), name: user.clone(), user: user.clone(), ssh_key: String::new() };
            if let Some(p) = members.iter().find_map(|hid| store::get_password(hid)) {
                store::set_password(&store::acct_key(&a.id), Some(&p));
            }
            for hid in &members {
                store::set_password(hid, None);
            }
            let aid = a.id.clone();
            home(|m| {
                m.accounts.push(a);
                m.hosts.iter_mut().filter(|h| members.contains(&h.id)).for_each(|h| h.account = aid.clone());
            });
            save_accounts();
            save_hosts();
            send_hosts();
            send(json!({"type": "account_saved", "id": aid, "grouped": members.len()}));
        }
        "pick_file" => {
            let field = s(&v, "field");
            if let Some(path) = pick_file(home(|m| m.hwnd)) {
                send(json!({"type": "picked", "field": field, "path": path}));
            }
        }
        "forget_password" => {
            store::set_password(&id, None);
            send_hosts();
        }
        "settings" => {
            if let Ok(st) = serde_json::from_value::<Settings>(v["settings"].clone()) {
                let old = home(|m| std::mem::replace(&mut m.settings, st.clone()));
                store::save_settings(&st);
                apply_theme(home(|m| m.hwnd));
                if old.zoom != st.zoom {
                    apply_zoom(st.zoom);
                }
                send(json!({"type": "settings", "settings": st}));
                // Restart servers only when their own settings changed (restarts drop viewers).
                if old.web != st.web {
                    apply_web(&st.web);
                }
                if old.share != st.share {
                    apply_share(&st.share);
                }
            }
        }
        "web_info" => send(web_info()),
        "share_info" => send(share_info()),
        "share_reset_pw" | "share_set_pw" => {
            let p = s(&v, "password");
            if s(&v, "cmd") == "share_set_pw" && p.len() >= 6 {
                store::share_password(Some(&p), false);
            } else {
                store::share_password(None, true);
            }
            crate::share::kick_all();
            let st = home(|m| m.settings.share.clone());
            apply_share(&st);
        }
        "share_kick" => crate::share::kick_all(),
        "web_tailscale" => {
            let (on, port) = (v["on"].as_bool().unwrap_or(false), home_web_port());
            std::thread::spawn(move || post(Event::TsServe(crate::tailscale::serve(port, on))));
        }
        "web_reset_pin" => {
            store::web_pin(true);
            crate::gateway::stop(); // sign everyone out
            let w = home(|m| m.settings.web.clone());
            apply_web(&w);
        }
        "scan" => scan(),
        "open" => {
            let target = match s(&v, "what").as_str() {
                "log" => crate::log::path(),
                "folder" => store::dir(),
                "screenshots" => {
                    let d = crate::ui::pictures_dir();
                    let _ = std::fs::create_dir_all(&d);
                    d
                }
                _ => return,
            };
            let _ = std::fs::create_dir_all(target.parent().unwrap_or(&target));
            let t = wide(&target.to_string_lossy());
            ShellExecuteW(null_mut(), wide("open").as_ptr(), t.as_ptr(), null(), null(), SW_SHOWNORMAL);
        }
        "url" => {
            let u = s(&v, "url");
            if u.starts_with("https://") || u.starts_with("x-apple") {
                ShellExecuteW(null_mut(), wide("open").as_ptr(), wide(&u).as_ptr(), null(), null(), SW_SHOWNORMAL);
            }
        }
        "minimize" => {
            ShowWindow(home(|m| m.hwnd), SW_MINIMIZE);
        }
        c => crate::log!("ipc: unknown command {c}"),
    }
}

/// Standard "Open" dialog for an SSH private key, starting in ~/.ssh.
unsafe fn pick_file(owner: HWND) -> Option<String> {
    use windows_sys::Win32::UI::Controls::Dialogs::*;
    let mut buf = vec![0u16; 1024];
    let filter = wide("SSH 개인 키\0id_*;*.pem;*.key\0모든 파일\0*.*\0");
    let start = std::env::var_os("USERPROFILE").map(|p| std::path::PathBuf::from(p).join(".ssh")).filter(|p| p.is_dir());
    let start_w = start.map(|p| wide(&p.to_string_lossy()));
    let title = wide("SSH 키 파일 선택");
    let mut ofn: OPENFILENAMEW = std::mem::zeroed();
    ofn.lStructSize = size_of::<OPENFILENAMEW>() as u32;
    ofn.hwndOwner = owner;
    ofn.lpstrFilter = filter.as_ptr();
    ofn.lpstrFile = buf.as_mut_ptr();
    ofn.nMaxFile = buf.len() as u32;
    ofn.lpstrTitle = title.as_ptr();
    ofn.lpstrInitialDir = start_w.as_ref().map_or(null(), |w| w.as_ptr());
    ofn.Flags = OFN_FILEMUSTEXIST | OFN_PATHMUSTEXIST | OFN_NOCHANGEDIR;
    if GetOpenFileNameW(&mut ofn) == 0 {
        return None;
    }
    let n = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
    Some(String::from_utf16_lossy(&buf[..n]))
}

// ---------------------------------------------------------------- share this PC

static SHARE_ERR: Mutex<String> = Mutex::new(String::new());
static SHARE_SEEN: Mutex<Vec<String>> = Mutex::new(Vec::new());

fn apply_share(s: &store::Share) {
    let _ = crate::share::ON_CHANGE.set(|| post(Event::Share));
    let r = if s.on {
        crate::share::start(crate::share::Config {
            port: s.port,
            password: store::share_password(None, false),
            ask: s.ask,
            cmd_as_ctrl: s.cmd_as_ctrl,
            view_only: s.view_only,
        })
    } else {
        crate::share::stop();
        Ok(())
    };
    *SHARE_ERR.lock().unwrap() = r.err().map(|e| e.to_string()).unwrap_or_default();
    send(share_info());
}

fn share_info() -> Value {
    let port = crate::share::running();
    let addrs: Vec<Value> = port
        .map(|p| crate::gateway::urls(p).into_iter().map(|u| json!({"addr": u.url.trim_start_matches("http://").trim_end_matches('/'), "kind": u.kind})).collect())
        .unwrap_or_default();
    let ts = TAILNET.lock().unwrap().clone().filter(|t| t.running).map(|t| t.me.dns).filter(|d| !d.is_empty());
    json!({
        "type": "share",
        "running": port.is_some(),
        "port": port,
        "addrs": addrs,
        "tailscale": ts.zip(port).map(|(d, p)| if p == 5900 { d } else { format!("{d}:{p}") }),
        "password": if port.is_some() { store::share_password(None, false) } else { String::new() },
        "viewers": crate::share::viewers(),
        "name": std::env::var("COMPUTERNAME").unwrap_or_default(),
        "error": *SHARE_ERR.lock().unwrap(),
    })
}

/// Tray tooltip says when someone is watching.
fn tray_tip(viewers: &[crate::share::Viewer]) {
    unsafe {
        let mut d = tray_data(home(|m| m.hwnd));
        d.uFlags = NIF_TIP;
        let tip = if viewers.is_empty() { crate::APP.to_string() } else { format!("{} — {}명이 이 PC를 보는 중", crate::APP, viewers.len()) };
        copy_w(&mut d.szTip, &tip);
        Shell_NotifyIconW(NIM_MODIFY, &d);
    }
}

// ---------------------------------------------------------------- web viewer

static WEB_ERR: Mutex<String> = Mutex::new(String::new());
static TS_ERR: Mutex<String> = Mutex::new(String::new());
static TAILNET: Mutex<Option<crate::tailscale::Status>> = Mutex::new(None);
static SERVING: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Start/stop the gateway to match the settings, then tell the page.
fn apply_web(w: &store::Web) {
    let _ = crate::gateway::ON_EDIT.set(|v| {
        let (tx, rx) = std::sync::mpsc::channel();
        post(Event::WebEdit(v, tx));
        rx.recv_timeout(Duration::from_secs(5)).is_ok()
    });
    WEB_PORT.store(w.port, Relaxed);
    let r = if w.on { crate::gateway::start(w.port) } else { crate::gateway::stop(); Ok(()) };
    *WEB_ERR.lock().unwrap() = r.err().map(|e| e.to_string()).unwrap_or_default();
    send(web_info());
}

fn web_info() -> Value {
    let port = crate::gateway::running();
    let urls = port.map(crate::gateway::urls).unwrap_or_default();
    let qr_of = |u: &str| {
        let code = qrcode::QrCode::new(u.as_bytes()).ok()?;
        Some(code.render::<qrcode::render::svg::Color>().min_dimensions(176, 176).quiet_zone(true).build())
    };
    let qr = urls.first().and_then(|u| qr_of(&u.url));
    // Over Tailscale: the MagicDNS name works from anywhere the phone runs Tailscale.
    let ts = TAILNET.lock().unwrap().clone().filter(|t| t.running);
    let serving = SERVING.load(Relaxed);
    let tailscale = match (&ts, port) {
        (Some(t), Some(p)) if !t.me.dns.is_empty() => {
            let url = if serving { format!("https://{}/", t.me.dns) } else { format!("http://{}:{p}/", t.me.dns) };
            json!({"url": url, "qr": qr_of(&url), "https": t.https, "serving": serving, "error": *TS_ERR.lock().unwrap()})
        }
        _ => Value::Null,
    };
    json!({
        "type": "web",
        "running": port.is_some(),
        "port": port,
        "urls": urls,
        "qr": qr,
        "pin": if port.is_some() { store::web_pin(false) } else { String::new() },
        "error": *WEB_ERR.lock().unwrap(),
        "tailscale": tailscale,
    })
}

// ---------------------------------------------------------------- sessions

unsafe fn connect(id: &str, typed: Option<String>, save: bool) {
    if home(|m| m.sessions.contains_key(id)) {
        return focus_session(id);
    }
    let Some(host) = home(|m| m.hosts.iter().find(|x| x.id == id).cloned()) else { return };
    let login = home(|m| store::resolve(&host, &m.accounts));
    if host.is_rdp() {
        return open_rdp(&host, &login, typed);
    }
    let pass = match typed.clone().or_else(|| login.pass.clone()) {
        Some(p) => p,
        None => {
            send(json!({"type": "need_password", "id": id}));
            return;
        }
    };
    let settings = home(|m| m.settings.clone());
    let launch = Launch {
        id: id.to_string(),
        title: host.label().to_string(),
        host: host.host.clone(),
        user: login.user.clone(),
        pass,
        opts: host.opts,
        ssh_key: login.ssh_key.clone(),
        key_pass: login.key_pass.clone().unwrap_or_default(),
        hotkey: settings.hotkey.clone(),
        toolbar: settings.toolbar.clone(),
        thumbnails: settings.thumbnails,
        ssh: host.ssh.clone(),
    };
    let exe = std::env::current_exe().unwrap_or_else(|_| "itda.exe".into());
    let child = Command::new(exe)
        .arg("--session")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .creation_flags(CREATE_NO_WINDOW)
        .spawn();
    let mut child = match child {
        Ok(c) => c,
        Err(e) => {
            send(json!({"type": "error", "id": id, "kind": "other", "msg": format!("세션을 시작할 수 없습니다: {e}")}));
            return;
        }
    };
    AllowSetForegroundWindow(child.id());
    if let Some(mut stdin) = child.stdin.take() {
        let _ = writeln!(stdin, "{}", serde_json::to_string(&launch).unwrap_or_default());
    }
    if let Some(out) = child.stdout.take() {
        let sid = id.to_string();
        std::thread::spawn(move || {
            for line in BufReader::new(out).lines().map_while(Result::ok) {
                if let Ok(v) = serde_json::from_str::<Value>(&line) {
                    post(Event::Session(sid.clone(), v));
                }
            }
            post(Event::Exit(sid));
        });
    }
    let save_pw = if save { typed } else { None };
    home(|m| {
        m.sessions.insert(id.to_string(), Sess { child, hwnd: 0, state: "connecting".into(), stage: "start".into(), save_pw });
    });
    send_session(id);
}

/// Windows Remote Desktop: hand mstsc the sign-in, then let it run on its own.
fn open_rdp(host: &Host, login: &store::Login, typed: Option<String>) {
    let target = host.host.trim();
    let name = target.rsplit_once(':').filter(|(_, p)| p.parse::<u16>().is_ok()).map_or(target, |(h, _)| h);
    if let Some(p) = typed.or_else(|| login.pass.clone()) {
        if !login.user.is_empty() {
            store::set_rdp_cred(name, &login.user, &p);
        }
    }
    let mut cmd = Command::new("mstsc.exe");
    cmd.arg(format!("/v:{target}"));
    if host.opts.fullscreen {
        cmd.arg("/f");
    }
    match cmd.spawn() {
        Ok(_) => {
            let id = host.id.clone();
            home(|m| m.hosts.iter_mut().filter(|h| h.id == id).for_each(|h| h.last_used = store::now()));
            save_hosts();
            send_hosts();
            send(json!({"type": "toast", "text": format!("{} — 원격 데스크톱을 열었습니다", host.label())}));
        }
        Err(e) => send(json!({"type": "error", "id": host.id, "kind": "other", "msg": format!("원격 데스크톱(mstsc)을 열 수 없습니다: {e}")})),
    }
}

fn stop_session(id: &str) {
    let hw = home(|m| m.sessions.get(id).map(|s| s.hwnd));
    match hw {
        Some(hwnd) if hwnd != 0 => unsafe {
            PostMessageW(hwnd as HWND, WM_CLOSE, 0, 0);
        },
        Some(_) => home(|m| {
            if let Some(s) = m.sessions.get_mut(id) {
                let _ = s.child.kill();
            }
        }),
        None => {}
    }
}

unsafe fn focus_session(id: &str) {
    if let Some(hw) = home(|m| m.sessions.get(id).map(|s| s.hwnd)).filter(|&h| h != 0) {
        let w = hw as HWND;
        if IsIconic(w) != 0 {
            ShowWindow(w, SW_RESTORE);
        }
        SetForegroundWindow(w);
    }
}

unsafe fn on_session(id: &str, v: Value) {
    match s(&v, "ev").as_str() {
        "stage" => {
            home(|m| {
                if let Some(x) = m.sessions.get_mut(id) {
                    x.stage = s(&v, "stage");
                }
            });
            send_session(id);
        }
        "connected" => {
            let pw = home(|m| {
                let x = m.sessions.get_mut(id)?;
                x.state = "connected".into();
                x.hwnd = v["hwnd"].as_i64().unwrap_or(0) as isize;
                x.save_pw.take()
            });
            if let Some(p) = pw {
                // A shared account keeps the password for all its connections.
                let key = home(|m| m.hosts.iter().find(|h| h.id == id).map(|h| h.account.clone())).filter(|a| !a.is_empty()).map(|a| store::acct_key(&a));
                store::set_password(key.as_deref().unwrap_or(id), Some(&p));
            }
            home(|m| {
                if let Some(h) = m.hosts.iter_mut().find(|x| x.id == id) {
                    h.last_used = store::now();
                    h.remote_name = s(&v, "name");
                    h.remote_size = format!("{}×{}", v["w"].as_u64().unwrap_or(0), v["h"].as_u64().unwrap_or(0));
                }
                m.online.insert(id.to_string(), true);
            });
            save_hosts();
            send_hosts();
            send_session(id);
        }
        "error" => {
            home(|m| m.sessions.remove(id));
            send_session(id);
            let kind = s(&v, "kind");
            let label = home(|m| m.hosts.iter().find(|x| x.id == id).map(|h| h.label().to_string())).unwrap_or_default();
            if kind == "lost" || kind == "auth" {
                notify(&format!("{label} 연결 종료"), &s(&v, "msg"));
            }
            ShowWindow(home(|m| m.hwnd), SW_SHOW);
            send(json!({"type": "error", "id": id, "kind": kind, "msg": s(&v, "msg")}));
        }
        "reconnecting" => {
            home(|m| {
                if let Some(x) = m.sessions.get_mut(id) {
                    x.state = "reconnecting".into();
                }
            });
            if v["n"].as_u64() == Some(1) {
                let label = home(|m| m.hosts.iter().find(|x| x.id == id).map(|h| h.label().to_string())).unwrap_or_default();
                notify(&format!("{label} 연결이 끊겼습니다"), "자동으로 다시 연결하는 중입니다.");
            }
            send_session(id);
        }
        "reconnected" => {
            home(|m| {
                if let Some(x) = m.sessions.get_mut(id) {
                    x.state = "connected".into();
                }
            });
            send_session(id);
        }
        "opts" => {
            if let Ok(o) = serde_json::from_value(v["opts"].clone()) {
                home(|m| m.hosts.iter_mut().filter(|x| x.id == id).for_each(|x| x.opts = o));
                save_hosts();
                send_hosts();
            }
        }
        "toolbar" => {
            let st = home(|m| {
                m.settings.toolbar = s(&v, "mode");
                m.settings.clone()
            });
            store::save_settings(&st);
            send(json!({"type": "settings", "settings": st}));
        }
        "thumb" => send(json!({"type": "thumb", "id": id, "v": thumb_version(id)})),
        "closed" => {}
        e => crate::log!("session {id}: unknown event {e}"),
    }
}

unsafe fn on_exit(id: &str) {
    let gone = home(|m| m.sessions.remove(id));
    if let Some(mut s) = gone {
        let _ = s.child.wait();
        if s.state == "connecting" {
            // Died before reporting anything useful.
            send(json!({"type": "error", "id": id, "kind": "other", "msg": "연결 프로세스가 예기치 않게 종료되었습니다"}));
        }
    }
    send_session(id);
}

unsafe fn drain() {
    let evs = std::mem::take(&mut *QUEUE.lock().unwrap());
    for e in evs {
        match e {
            Event::Session(id, v) if id == "__ipc" => on_ipc(v),
            Event::Session(id, v) => on_session(&id, v),
            Event::Exit(id) => on_exit(&id),
            Event::Online(map) => {
                let changed = home(|m| {
                    let mut c = false;
                    for (k, v) in map {
                        // A running session is proof enough.
                        let v = v || m.sessions.get(&k).is_some_and(|s| s.state == "connected");
                        c |= m.online.insert(k, v) != Some(v);
                    }
                    c
                });
                if changed {
                    let o = home(|m| json!(m.online));
                    send(json!({"type": "online", "online": o}));
                }
            }
            Event::Found(list) => {
                home(|m| m.found = list.clone());
                send(json!({"type": "found", "found": list}));
            }
            Event::Tailnet(st, serving) => {
                SERVING.store(serving, Relaxed);
                send(json!({"type": "tailnet", "tailnet": st}));
                *TAILNET.lock().unwrap() = Some(st);
                if crate::gateway::running().is_some() {
                    send(web_info());
                }
            }
            Event::Share => {
                let now = crate::share::viewers();
                let fresh: Vec<String> = now.iter().map(|v| v.peer.clone()).filter(|p| !SHARE_SEEN.lock().unwrap().contains(p)).collect();
                *SHARE_SEEN.lock().unwrap() = now.iter().map(|v| v.peer.clone()).collect();
                for p in fresh {
                    notify("누군가 이 PC를 보고 있습니다", &format!("{p}에서 연결했습니다. 알림 영역의 Itda에서 끊을 수 있습니다."));
                }
                tray_tip(&now);
                send(share_info());
            }
            Event::TsServe(r) => {
                *TS_ERR.lock().unwrap() = r.err().unwrap_or_default();
                scan_tailnet();
                send(web_info());
            }
            Event::WebEdit(v, done) => {
                on_ipc(v);
                let _ = done.send(());
            }
        }
    }
}

// ---------------------------------------------------------------- reachability & discovery

static PROBE_WAKE: Mutex<bool> = Mutex::new(false);
static PROBE_CV: std::sync::Condvar = std::sync::Condvar::new();

fn probe_now() {
    *PROBE_WAKE.lock().unwrap() = true;
    PROBE_CV.notify_all();
}

/// Every 20 s (or on demand): is each Mac's Screen Sharing port answering?
fn start_probes() {
    std::thread::spawn(|| loop {
        // hosts.json is small and only written by the UI thread (atomically): just read it.
        // SSH hosts are reachable when Remote Login answers; the Screen Sharing port may be firewalled.
        let targets: Vec<(String, String)> = store::load_hosts()
            .into_iter()
            .map(|h| {
                let addr = if h.is_rdp() {
                    let t = h.host.trim();
                    if t.rsplit_once(':').is_some_and(|(_, p)| p.parse::<u16>().is_ok()) { t.to_string() } else { format!("{t}:3389") }
                } else if h.ssh.on {
                    let (host, _) = crate::tunnel::split_host(&h.host, 5900);
                    let port = if h.ssh.port == 0 { 22 } else { h.ssh.port };
                    if host.contains(':') { format!("[{host}]:{port}") } else { format!("{host}:{port}") }
                } else {
                    crate::rfb::addr(&h.host)
                };
                (h.id, addr)
            })
            .collect();
        let results: HashMap<String, bool> = std::thread::scope(|sc| {
            let hs: Vec<_> = targets
                .iter()
                .map(|(id, addr)| {
                    sc.spawn(move || {
                        let ok = addr.to_socket_addrs().is_ok_and(|mut it| {
                            it.any(|sa| TcpStream::connect_timeout(&sa, Duration::from_millis(1500)).is_ok())
                        });
                        (id.clone(), ok)
                    })
                })
                .collect();
            hs.into_iter().filter_map(|h| h.join().ok()).collect()
        });
        post(Event::Online(results));
        let g = PROBE_WAKE.lock().unwrap();
        let (mut g, _) = PROBE_CV.wait_timeout_while(g, Duration::from_secs(20), |w| !*w).unwrap();
        *g = false;
    });
}

fn scan() {
    scan_tailnet();
    std::thread::spawn(|| {
        let found = crate::mdns::browse(Duration::from_millis(1500));
        crate::log!("bonjour: {} found {:?}", found.len(), found.iter().map(|f| &f.host).collect::<Vec<_>>());
        post(Event::Found(found));
    });
}

/// Devices on the user's tailnet: reachable from anywhere, not just this Wi-Fi.
fn scan_tailnet() {
    std::thread::spawn(|| {
        let st = crate::tailscale::status();
        if !st.installed {
            return;
        }
        let port = home_web_port();
        let serving = st.running && crate::tailscale::serving(port);
        crate::log!("tailscale: running={} peers={} https={} serving={serving}", st.running, st.peers.len(), st.https);
        post(Event::Tailnet(st, serving));
    });
}

static WEB_PORT: std::sync::atomic::AtomicU16 = std::sync::atomic::AtomicU16::new(8780);

fn home_web_port() -> u16 {
    WEB_PORT.load(Relaxed)
}

// ---------------------------------------------------------------- tray

unsafe fn tray_data(h: HWND) -> NOTIFYICONDATAW {
    let mut d: NOTIFYICONDATAW = std::mem::zeroed();
    d.cbSize = size_of::<NOTIFYICONDATAW>() as u32;
    d.hWnd = h;
    d.uID = 1;
    d
}

fn copy_w(dst: &mut [u16], s: &str) {
    let w: Vec<u16> = s.encode_utf16().take(dst.len() - 1).collect();
    dst[..w.len()].copy_from_slice(&w);
    dst[w.len()] = 0;
}

unsafe fn tray_add(h: HWND) {
    let mut d = tray_data(h);
    d.uFlags = NIF_ICON | NIF_MESSAGE | NIF_TIP;
    d.uCallbackMessage = WM_TRAY;
    d.hIcon = LoadIconW(GetModuleHandleW(null()), 1 as _);
    if d.hIcon.is_null() {
        d.hIcon = LoadIconW(null_mut(), IDI_APPLICATION);
    }
    copy_w(&mut d.szTip, crate::APP);
    Shell_NotifyIconW(NIM_ADD, &d);
}

unsafe fn tray_remove(h: HWND) {
    let d = tray_data(h);
    Shell_NotifyIconW(NIM_DELETE, &d);
}

/// Windows notification (balloon from the tray icon).
fn notify(title: &str, text: &str) {
    if !home(|m| m.settings.notify) {
        return;
    }
    unsafe {
        let h = home(|m| m.hwnd);
        let mut d = tray_data(h);
        d.uFlags = NIF_INFO;
        copy_w(&mut d.szInfoTitle, title);
        copy_w(&mut d.szInfo, if text.is_empty() { " " } else { text });
        d.dwInfoFlags = NIIF_INFO;
        Shell_NotifyIconW(NIM_MODIFY, &d);
    }
}

unsafe fn show_home(h: HWND) {
    ShowWindow(h, SW_SHOW);
    if IsIconic(h) != 0 {
        ShowWindow(h, SW_RESTORE);
    }
    SetForegroundWindow(h);
}

unsafe fn tray_menu(h: HWND) {
    let m = CreatePopupMenu();
    AppendMenuW(m, MF_STRING | MF_DEFAULT, TRAY_OPEN, wide("Itda 열기").as_ptr());
    let mut recent = home(|m| m.hosts.clone());
    recent.sort_by(|a, b| b.last_used.cmp(&a.last_used));
    recent.truncate(6);
    if !recent.is_empty() {
        AppendMenuW(m, MF_SEPARATOR, 0, null());
        for (i, x) in recent.iter().enumerate() {
            let live = home(|mm| mm.sessions.contains_key(&x.id));
            let label = format!("{}{}", x.label(), if live { "  (연결됨)" } else { "" });
            AppendMenuW(m, MF_STRING, TRAY_HOST0 + i, wide(&label).as_ptr());
        }
    }
    AppendMenuW(m, MF_SEPARATOR, 0, null());
    let sharing = home(|m| m.settings.web.on || m.settings.share.on);
    AppendMenuW(m, MF_STRING, TRAY_SHARE, wide(if sharing { "공유 중 — QR·주소 보기" } else { "다른 기기에서 보기 (공유)…" }).as_ptr());
    AppendMenuW(m, MF_STRING, TRAY_QUIT, wide("종료").as_ptr());
    let mut pt = POINT { x: 0, y: 0 };
    GetCursorPos(&mut pt);
    SetForegroundWindow(h);
    let cmd = TrackPopupMenu(m, TPM_RETURNCMD | TPM_RIGHTBUTTON, pt.x, pt.y, 0, h, null()) as usize;
    DestroyMenu(m);
    match cmd {
        TRAY_OPEN => show_home(h),
        TRAY_SHARE => {
            show_home(h);
            send(json!({"type": "go", "page": "share"}));
        }
        TRAY_QUIT => {
            DestroyWindow(h);
        }
        c if c >= TRAY_HOST0 && c < TRAY_HOST0 + recent.len() => {
            let id = recent[c - TRAY_HOST0].id.clone();
            if store::get_password(&id).is_none() && !home(|m| m.sessions.contains_key(&id)) {
                show_home(h); // the page asks for the password
            }
            connect(&id, None, false);
        }
        _ => {}
    }
}

// ---------------------------------------------------------------- window

unsafe extern "system" fn wndproc(h: HWND, m: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    match m {
        WM_SIZE => resize_webview(h),
        WM_EVENT => drain(),
        WM_COPYDATA => {
            let cd = &*(lp as *const windows_sys::Win32::System::DataExchange::COPYDATASTRUCT);
            if cd.dwData == 1 && !cd.lpData.is_null() && cd.cbData < 4096 {
                let q = String::from_utf8_lossy(std::slice::from_raw_parts(cd.lpData as *const u8, cd.cbData as usize)).into_owned();
                show_home(h);
                send(json!({"type": "quick", "address": q}));
            }
            return 1;
        }
        WM_TRAY => match (lp & 0xffff) as u32 {
            WM_LBUTTONUP | WM_LBUTTONDBLCLK => show_home(h),
            WM_RBUTTONUP | WM_CONTEXTMENU => tray_menu(h),
            _ => {}
        },
        WM_GETMINMAXINFO => {
            let mi = &mut *(lp as *mut MINMAXINFO);
            let d = GetDpiForWindow(h).max(96) as i32;
            mi.ptMinTrackSize = POINT { x: 720 * d / 96, y: 500 * d / 96 };
        }
        WM_SETTINGCHANGE => {
            if lp != 0 {
                let p = lp as *const u16;
                let mut n = 0;
                while *p.add(n) != 0 && n < 64 {
                    n += 1;
                }
                if String::from_utf16_lossy(std::slice::from_raw_parts(p, n)) == "ImmersiveColorSet" {
                    apply_theme(h);
                    send(json!({"type": "system_theme", "dark": system_dark()}));
                }
            }
            return DefWindowProcW(h, m, wp, lp);
        }
        WM_DPICHANGED => {
            let r = &*(lp as *const RECT);
            SetWindowPos(h, null_mut(), r.left, r.top, r.right - r.left, r.bottom - r.top, SWP_NOZORDER | SWP_NOACTIVATE);
        }
        WM_CLOSE => {
            let (tray, hinted) = home(|m| (m.settings.tray, m.settings.tray_hint_shown));
            if tray {
                ShowWindow(h, SW_HIDE);
                if !hinted {
                    let st = home(|m| {
                        m.settings.tray_hint_shown = true;
                        m.settings.clone()
                    });
                    store::save_settings(&st);
                    notify("Itda는 계속 실행 중입니다", "알림 영역 아이콘에서 다시 열거나 종료할 수 있습니다.");
                }
            } else {
                DestroyWindow(h);
            }
        }
        WM_DESTROY => {
            let wv = WEBVIEW.with(|w| w.borrow_mut().take());
            drop(wv);
            PostQuitMessage(0);
        }
        _ if m != 0 && m == TRAY_MSG.load(Relaxed) => tray_add(h),
        _ => return DefWindowProcW(h, m, wp, lp),
    }
    0
}


/// Test-only stand-in for WebView2 (`ITDA_TEST_HTTP=<port>`): serves the same page on
/// 127.0.0.1 and carries the same IPC over HTTP, so a headless browser can drive the real
/// home logic where WebView2 can't run (e.g. a non-interactive window station).
mod test_bridge {
    use std::io::{BufRead, BufReader, Read, Write};
    use std::net::{TcpListener, TcpStream};
    use std::sync::atomic::{AtomicBool, Ordering::Relaxed};
    use std::sync::{Condvar, Mutex};
    use std::time::Duration;

    static ACTIVE: AtomicBool = AtomicBool::new(false);
    static OUTBOX: Mutex<Vec<String>> = Mutex::new(Vec::new());
    static READY: Condvar = Condvar::new();

    const SHIM: &str = "window.ipc={postMessage:m=>fetch('/ipc',{method:'POST',body:m})};\
        (async()=>{for(;;){try{const r=await fetch('/poll');for(const m of await r.json())window.itda&&itda.recv(m)}\
        catch(e){await new Promise(r=>setTimeout(r,300))}}})();";

    pub fn active() -> bool {
        ACTIVE.load(Relaxed)
    }

    pub fn push(msg: String) {
        OUTBOX.lock().unwrap().push(msg);
        READY.notify_all();
    }

    pub fn start(port: u16) {
        ACTIVE.store(true, Relaxed);
        let l = TcpListener::bind(("127.0.0.1", port)).expect("test bridge port");
        crate::log!("test bridge on http://127.0.0.1:{port}/");
        std::thread::spawn(move || {
            for c in l.incoming().flatten() {
                std::thread::spawn(move || {
                    let _ = handle(c);
                });
            }
        });
    }

    fn handle(mut c: TcpStream) -> std::io::Result<()> {
        let mut r = BufReader::new(c.try_clone()?);
        let mut line = String::new();
        r.read_line(&mut line)?;
        let mut len = 0usize;
        loop {
            let mut h = String::new();
            r.read_line(&mut h)?;
            if h.trim().is_empty() {
                break;
            }
            if let Some(v) = h.to_ascii_lowercase().strip_prefix("content-length:") {
                len = v.trim().parse().unwrap_or(0);
            }
        }
        let mut body = vec![0; len];
        r.read_exact(&mut body)?;
        let path = line.split_whitespace().nth(1).unwrap_or("/").split('?').next().unwrap_or("/").to_string();
        let (status, mime, data): (&str, &str, Vec<u8>) = match (line.starts_with("POST"), path.as_str()) {
            (true, "/ipc") => {
                if let Ok(v) = serde_json::from_slice(&body) {
                    super::QUEUE.lock().unwrap().push(super::Event::Session("__ipc".into(), v));
                    unsafe { super::PostMessageW(super::HWND_HOME.load(Relaxed) as _, super::WM_EVENT, 0, 0) };
                }
                ("204 No Content", "text/plain", Vec::new())
            }
            (false, "/poll") => {
                let g = OUTBOX.lock().unwrap();
                let (mut g, _) = READY.wait_timeout_while(g, Duration::from_secs(10), |o| o.is_empty()).unwrap();
                let msgs: Vec<String> = std::mem::take(&mut *g);
                ("200 OK", "application/json", format!("[{}]", msgs.join(",")).into_bytes())
            }
            (false, p) => match super::asset(p) {
                Some((b, m)) if m.starts_with("text/html") => {
                    let html = String::from_utf8_lossy(&b).replace("<script src=\"app.js\">", &format!("<script>{SHIM}</script><script src=\"app.js\">"));
                    ("200 OK", m, html.into_bytes())
                }
                Some((b, m)) => ("200 OK", m, b.into_owned()),
                None => ("404 Not Found", "text/plain", b"not found".to_vec()),
            },
            _ => ("405 Method Not Allowed", "text/plain", Vec::new()),
        };
        write!(c, "HTTP/1.1 {status}\r\nContent-Type: {mime}\r\nContent-Length: {}\r\nCache-Control: no-store\r\nConnection: close\r\n\r\n", data.len())?;
        c.write_all(&data)
    }
}