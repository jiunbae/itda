//! Windows side of the home: the WebView2 window, tray icon, notifications and the
//! `itda --session` processes.

use super::{connect, drain, home, is_dark, send, Event, QUEUE};
use crate::session::Launch;
use crate::store::{self, Host, Settings};
use crate::ui::wide;
use serde_json::{json, Value};
use std::borrow::Cow;
use std::cell::RefCell;
use std::io::{BufRead, BufReader, Write};
use std::num::NonZeroIsize;
use std::os::windows::process::CommandExt;
use std::process::{Child, Command, Stdio};
use std::ptr::{null, null_mut};
use std::sync::atomic::{AtomicIsize, Ordering::Relaxed};
use windows_sys::Win32::Foundation::*;
use windows_sys::Win32::Graphics::Dwm::DwmSetWindowAttribute;
use windows_sys::Win32::Graphics::Gdi::*;
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::System::Registry::{RegGetValueW, HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE, RRF_RT_REG_DWORD};
use windows_sys::Win32::System::Threading::CreateMutexW;
use windows_sys::Win32::UI::HiDpi::GetDpiForWindow;
use windows_sys::Win32::UI::Shell::*;
use windows_sys::Win32::UI::WindowsAndMessaging::*;
use wry::raw_window_handle::{HandleError, HasWindowHandle, RawWindowHandle, Win32WindowHandle, WindowHandle};
use wry::{http, Rect, WebView, WebViewBuilder, WebViewBuilderExtWindows};

pub const PLATFORM: &str = "windows";

const WM_EVENT: u32 = WM_APP + 20;
const WM_TRAY: u32 = WM_APP + 21;
const TRAY_OPEN: usize = 1;
const TRAY_QUIT: usize = 2;
const TRAY_SHARE: usize = 3;
const TRAY_HOST0: usize = 100;
const CREATE_NO_WINDOW: u32 = 0x0800_0000;
const DWMWA_USE_IMMERSIVE_DARK_MODE: u32 = 20;

static HWND_HOME: AtomicIsize = AtomicIsize::new(0);
static TRAY_MSG: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);

thread_local! {
    /// WebView2 calls can re-enter the window procedure, so the webview is only ever
    /// borrowed shared (except when created and dropped).
    static FILE_HOSTS: RefCell<std::collections::HashMap<String, String>> = RefCell::new(std::collections::HashMap::new());
    static FILE_PANES: RefCell<std::collections::HashMap<String, (isize, WebView)>> = RefCell::new(std::collections::HashMap::new());
    static WEBVIEW: RefCell<Option<WebView>> = const { RefCell::new(None) };
}

fn hwnd() -> HWND {
    HWND_HOME.load(Relaxed) as HWND
}

/// Have the UI thread drain the queue.
pub fn wake() {
    unsafe { PostMessageW(hwnd(), WM_EVENT, 0, 0) };
}

/// A connection: its `itda --session` process and, once it shows, its window.
pub struct Proc {
    child: Child,
    hwnd: isize,
}

struct Win(HWND);

impl HasWindowHandle for Win {
    fn window_handle(&self) -> Result<WindowHandle<'_>, HandleError> {
        let raw = Win32WindowHandle::new(NonZeroIsize::new(self.0 as isize).ok_or(HandleError::Unavailable)?);
        Ok(unsafe { WindowHandle::borrow_raw(RawWindowHandle::Win32(raw)) })
    }
}

pub fn system_dark() -> bool {
    let mut v: u32 = 1;
    let mut n = 4u32;
    let key = wide(r"Software\Microsoft\Windows\CurrentVersion\Themes\Personalize");
    let val = wide("AppsUseLightTheme");
    unsafe { RegGetValueW(HKEY_CURRENT_USER, key.as_ptr(), val.as_ptr(), RRF_RT_REG_DWORD, null_mut(), &mut v as *mut u32 as _, &mut n) };
    v == 0
}

pub fn apply_theme(dark: bool) {
    let dark = dark as BOOL;
    unsafe { DwmSetWindowAttribute(hwnd(), DWMWA_USE_IMMERSIVE_DARK_MODE, &dark as *const BOOL as _, 4) };
}

/// Remote Desktop accepts connections (Settings → System → Remote Desktop).
pub fn rdp_enabled() -> bool {
    let mut v: u32 = 1;
    let mut n = 4u32;
    let key = wide(r"SYSTEM\CurrentControlSet\Control\Terminal Server");
    let val = wide("fDenyTSConnections");
    let r = unsafe { RegGetValueW(HKEY_LOCAL_MACHINE, key.as_ptr(), val.as_ptr(), RRF_RT_REG_DWORD, null_mut(), &mut v as *mut u32 as _, &mut n) };
    r == 0 && v == 0
}

pub fn computer_name() -> String {
    std::env::var("COMPUTERNAME").unwrap_or_default()
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
        super::init(quick);
        let dark = home(|m| is_dark(&m.settings));

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
        apply_theme(dark);
        let dpi = GetDpiForWindow(h) as i32;
        let (ww, wh) = (1080 * dpi / 96, 720 * dpi / 96);
        let mut wa: RECT = std::mem::zeroed();
        SystemParametersInfoW(SPI_GETWORKAREA, 0, &mut wa as *mut _ as _, 0);
        let (ww, wh) = (ww.min(wa.right - wa.left), wh.min(wa.bottom - wa.top));
        SetWindowPos(h, null_mut(), (wa.left + wa.right - ww) / 2, (wa.top + wa.bottom - wh) / 2, ww, wh, SWP_NOZORDER);

        let bridge = std::env::var("ITDA_TEST_HTTP").ok().and_then(|p| p.parse::<u16>().ok());
        match bridge.map(|p| Ok(super::test_bridge::start(p))).unwrap_or_else(|| build_webview(h, dark, None, None, None).map(|wv| WEBVIEW.with(|w| *w.borrow_mut() = Some(wv)))) {
            Ok(()) => {}
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
        super::started();

        let mut msg: MSG = std::mem::zeroed();
        while GetMessageW(&mut msg, null_mut(), 0, 0) > 0 {
            TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
        tray_remove(h);
    }
}

/// Files dragged past the file browser's edge go on as a native drag to Explorer.
pub(super) fn native_file_drag(v: Value) {
    let id = crate::files::scoped_id(v["id"].as_str().unwrap_or(""), &v["window"]);
    if let Err(e) = crate::files_win::drag(&v, |m| super::post(Event::Page(m))) {
        super::post(Event::Page(json!({"type":"files_native_drag_end","id":id,"error":e})));
    }
}

fn build_webview(h: HWND, dark: bool, file: Option<&str>, file_window: Option<&str>, file_path: Option<&str>) -> wry::Result<WebView> {
    let data = std::env::var_os("LOCALAPPDATA").map(std::path::PathBuf::from).unwrap_or_else(std::env::temp_dir).join(crate::APP).join("WebView2");
    let mut ctx = wry::WebContext::new(Some(data));
    let bg = if dark { (32, 32, 32, 255) } else { (243, 243, 243, 255) };
    let devtools = std::env::var_os("ITDA_DEVTOOLS").is_some();
    // Test hook: expose the DevTools protocol so the UI can be driven and captured.
    let mut args = "--disable-features=msWebOOUI,msPdfOOUI,msSmartScreenProtection".to_string();
    if let Ok(port) = std::env::var("ITDA_DEBUG_PORT") {
        args.push_str(&format!(" --remote-debugging-port={port}"));
    }
    let drop_id = file_window.map(str::to_string);
    WebViewBuilder::new_with_web_context(&mut ctx)
        .with_additional_browser_args(args)
        .with_custom_protocol("itda".into(), |_id, req| {
            let resp = match super::asset(req.uri().path()) {
                Some((body, mime)) => http::Response::builder().header("Content-Type", mime).header("Cache-Control", "no-store").body(body),
                None => http::Response::builder().status(404).body(Cow::Borrowed(&b"not found"[..])),
            };
            resp.unwrap_or_else(|_| http::Response::new(Cow::Borrowed(&b""[..])))
        })
        .with_drag_drop_handler(move |event| {
            if let wry::DragDropEvent::Drop { paths, .. } = event {
                super::post(Event::Page(json!({"type":"files_drop","window":drop_id,"paths":paths})));
            }
            true
        })
        .with_ipc_handler(|req| {
            let body = req.into_body();
            match serde_json::from_str::<Value>(&body) {
                Ok(v) => {
                    // Handle after this callback returns: WebView2 must not be re-entered from here.
                    QUEUE.lock().unwrap().push(Event::Session("__ipc".into(), v));
                    wake();
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
                open(&url);
            }
            false
        })
        .with_new_window_req_handler(|_url, _features| wry::NewWindowResponse::Deny)
        .with_url(format!("itda://localhost/index.html{}", file.map(|id| {
            let host = id.bytes().map(|b| format!("%{b:02X}")).collect::<String>();
            format!("?files={host}&fileWindow={}", file_window.unwrap_or(""))
        }).unwrap_or_default()))
        .with_background_color(bg)
        .with_initialization_script(&format!("window.itdaNativeDrag=true;window.ITDA_FILE_PATH={};", json!(file_path)))
        .with_devtools(devtools)
        .with_hotkeys_zoom(false)
        .with_browser_accelerator_keys(false)
        .with_default_context_menus(devtools)
        .build_as_child(&Win(h))
}

/// The home's own zoom (percent). WebView2's zoom shrinks the CSS viewport, so the page's
/// narrow-window layout takes over at large sizes.
pub fn apply_zoom(pct: u16) {
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
    if h != hwnd() {
        FILE_PANES.with(|p| { for (wh, wv) in p.borrow().values() { if *wh == h as isize { let _ = wv.set_bounds(Rect { position: wry::dpi::PhysicalPosition::new(0, 0).into(), size: wry::dpi::PhysicalSize::new(rc.right as u32, rc.bottom as u32).into() }); } } });
        return;
    }
    WEBVIEW.with(|w| {
        if let Some(wv) = w.borrow().as_ref() {
            let _ = wv.set_bounds(Rect {
                position: wry::dpi::PhysicalPosition::new(0, 0).into(),
                size: wry::dpi::PhysicalSize::new(rc.right as u32, rc.bottom as u32).into(),
            });
        }
    });
}

pub fn file_windows() -> Vec<String> {
    FILE_HOSTS.with(|p| p.borrow().values().cloned().collect())
}

pub fn files_window(id: &str, close: bool) { file_window(id, close, false, None); }
pub fn files_new_window_at(id: &str, path: Option<&str>) { file_window(id, false, true, path); }
pub fn files_window_context(window: &str, id: &str) {
    let h = FILE_PANES.with(|p| p.borrow().get(window).map(|(h,_)| *h));
    if let Some(h) = h {
        let title = home(|m| m.hosts.iter().find(|h| h.id == id).map(|h| format!("{} — 파일", h.label())).unwrap_or_else(|| "네트워크 — Itda".into()));
        unsafe { SetWindowTextW(h as HWND, wide(&title).as_ptr()); }
        FILE_HOSTS.with(|p| {p.borrow_mut().insert(window.into(), id.into());}); super::send_hosts();
    }
}
fn file_window(id: &str, close: bool, new: bool, path: Option<&str>) {
    let key = if close { Some(id.to_string()) } else if new { None } else { FILE_HOSTS.with(|p| p.borrow().iter().find(|(_,host)| *host == id).map(|(key,_)| key.clone())) };
    let existing = FILE_PANES.with(|p| key.as_ref().and_then(|key| p.borrow().get(key).map(|(h, _)| *h)));
    unsafe {
        if let Some(h) = existing {
            if close { DestroyWindow(h as HWND); } else { ShowWindow(h as HWND, SW_RESTORE); SetForegroundWindow(h as HWND); }
            return;
        }
        if close { return; }
        let window_id = crate::store::new_id();
        let title = home(|m| m.hosts.iter().find(|h| h.id == id).map(|h| format!("{} — 파일", h.label())).unwrap_or_else(|| "네트워크 — Itda".into()));
        let h = CreateWindowExW(0, wide("itda_home").as_ptr(), wide(&title).as_ptr(), WS_OVERLAPPEDWINDOW, CW_USEDEFAULT, CW_USEDEFAULT, 1000, 740, null_mut(), null_mut(), GetModuleHandleW(null()), null());
        if h.is_null() { return; }
        match build_webview(h, home(|m| is_dark(&m.settings)), Some(id), Some(&window_id), path) {
            Ok(view) => { FILE_HOSTS.with(|p| {p.borrow_mut().insert(window_id.clone(), id.into());}); FILE_PANES.with(|p| { p.borrow_mut().insert(window_id, (h as isize, view)); }); resize_webview(h); ShowWindow(h, SW_SHOW); SetForegroundWindow(h); super::send_hosts(); }
            Err(e) => { crate::log!("files webview: {e}"); DestroyWindow(h); }
        }
    }
}

pub fn eval(js: &str) {
    FILE_PANES.with(|p| { for (_, view) in p.borrow().values() { let _ = view.evaluate_script(js); } });
    WEBVIEW.with(|w| {
        if let Some(wv) = w.borrow().as_ref() {
            let _ = wv.evaluate_script(js);
        }
    });
}

/// Open a file, folder or https link with its default app.
pub fn open(target: &str) {
    let t = wide(target);
    unsafe { ShellExecuteW(null_mut(), wide("open").as_ptr(), t.as_ptr(), null(), null(), SW_SHOWNORMAL) };
}

pub fn minimize() {
    unsafe { ShowWindow(hwnd(), SW_MINIMIZE) };
}

pub fn screenshots_dir() -> std::path::PathBuf {
    crate::ui::pictures_dir()
}

/// Standard "Open" dialog for an SSH private key, starting in ~/.ssh.
pub fn pick_file(field: String) {
    use windows_sys::Win32::UI::Controls::Dialogs::*;
    let mut buf = vec![0u16; 1024];
    let filter = wide("SSH 개인 키\0id_*;*.pem;*.key\0모든 파일\0*.*\0");
    let start = std::env::var_os("USERPROFILE").map(|p| std::path::PathBuf::from(p).join(".ssh")).filter(|p| p.is_dir());
    let start_w = start.map(|p| wide(&p.to_string_lossy()));
    let title = wide("SSH 키 파일 선택");
    let mut ofn: OPENFILENAMEW = unsafe { std::mem::zeroed() };
    ofn.lStructSize = size_of::<OPENFILENAMEW>() as u32;
    ofn.hwndOwner = hwnd();
    ofn.lpstrFilter = filter.as_ptr();
    ofn.lpstrFile = buf.as_mut_ptr();
    ofn.nMaxFile = buf.len() as u32;
    ofn.lpstrTitle = title.as_ptr();
    ofn.lpstrInitialDir = start_w.as_ref().map_or(null(), |w| w.as_ptr());
    ofn.Flags = OFN_FILEMUSTEXIST | OFN_PATHMUSTEXIST | OFN_NOCHANGEDIR;
    if unsafe { GetOpenFileNameW(&mut ofn) } == 0 {
        return;
    }
    let n = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
    send(json!({"type": "picked", "field": field, "path": String::from_utf16_lossy(&buf[..n])}));
}

// ---------------------------------------------------------------- sessions

pub fn spawn_session(host: &Host, login: &store::Login, pass: String, settings: &Settings) -> Result<Proc, String> {
    let id = host.id.clone();
    let launch = Launch {
        id: id.clone(),
        title: host.label().to_string(),
        host: host.host.clone(),
        user: login.user.clone(),
        pass,
        opts: host.opts,
        ssh_key: login.ssh_key.clone(),
        key_pass: login.key_pass.clone().unwrap_or_default(),
        hotkey: settings.hotkey.clone(),
        toolbar: settings.toolbar.clone(),
        mac_scroll: settings.mac_scroll.clone(),
        scroll_accel: settings.scroll_accel,
        thumbnails: settings.thumbnails,
        ssh: host.ssh.clone(),
    };
    let exe = std::env::current_exe().unwrap_or_else(|_| "itda.exe".into());
    let mut child = Command::new(exe)
        .arg("--session")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .creation_flags(CREATE_NO_WINDOW)
        .spawn()
        .map_err(|e| e.to_string())?;
    unsafe { AllowSetForegroundWindow(child.id()) };
    if let Some(mut stdin) = child.stdin.take() {
        let _ = writeln!(stdin, "{}", serde_json::to_string(&launch).unwrap_or_default());
    }
    if let Some(out) = child.stdout.take() {
        std::thread::spawn(move || {
            for line in BufReader::new(out).lines().map_while(Result::ok) {
                if let Ok(v) = serde_json::from_str::<Value>(&line) {
                    super::post(Event::Session(id.clone(), v));
                }
            }
            super::post(Event::Exit(id));
        });
    }
    Ok(Proc { child, hwnd: 0 })
}

pub fn session_connected(p: &mut Proc, v: &Value, _label: &str) {
    p.hwnd = v["hwnd"].as_i64().unwrap_or(0) as isize;
}

pub fn stop_session(p: &mut Proc) {
    if p.hwnd != 0 {
        unsafe { PostMessageW(p.hwnd as HWND, WM_CLOSE, 0, 0) };
    } else {
        let _ = p.child.kill();
    }
}

pub fn focus_session(p: &Proc) {
    if p.hwnd != 0 {
        let w = p.hwnd as HWND;
        unsafe {
            if IsIconic(w) != 0 {
                ShowWindow(w, SW_RESTORE);
            }
            SetForegroundWindow(w);
        }
    }
}

/// Wait for the process; true: it ending on its own is worth reporting.
pub fn reap(mut p: Proc) -> bool {
    let _ = p.child.wait();
    true
}

/// Windows Remote Desktop: hand mstsc the sign-in, then let it run on its own.
pub fn open_rdp(host: &Host, login: &store::Login, typed: Option<String>) -> std::io::Result<()> {
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
    cmd.spawn().map(drop)
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

/// Tray tooltip says when someone is watching.
pub fn tray_tip(viewers: usize) {
    unsafe {
        let mut d = tray_data(hwnd());
        d.uFlags = NIF_TIP;
        let tip = if viewers == 0 { crate::APP.to_string() } else { format!("{} — {viewers}명이 이 PC를 보는 중", crate::APP) };
        copy_w(&mut d.szTip, &tip);
        Shell_NotifyIconW(NIM_MODIFY, &d);
    }
}

/// Windows notification (balloon from the tray icon).
pub fn notify(title: &str, text: &str) {
    unsafe {
        let mut d = tray_data(hwnd());
        d.uFlags = NIF_INFO;
        copy_w(&mut d.szInfoTitle, title);
        copy_w(&mut d.szInfo, if text.is_empty() { " " } else { text });
        d.dwInfoFlags = NIIF_INFO;
        Shell_NotifyIconW(NIM_MODIFY, &d);
    }
}

pub fn show_home() {
    let h = hwnd();
    unsafe {
        ShowWindow(h, SW_SHOW);
        if IsIconic(h) != 0 {
            ShowWindow(h, SW_RESTORE);
        }
        SetForegroundWindow(h);
    }
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
        TRAY_OPEN => show_home(),
        TRAY_SHARE => {
            show_home();
            send(json!({"type": "go", "page": "share"}));
        }
        TRAY_QUIT => {
            DestroyWindow(h);
        }
        c if c >= TRAY_HOST0 && c < TRAY_HOST0 + recent.len() => {
            let id = recent[c - TRAY_HOST0].id.clone();
            if store::get_password(&id).is_none() && !home(|m| m.sessions.contains_key(&id)) {
                show_home(); // the page asks for the password
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
                show_home();
                send(json!({"type": "quick", "address": q}));
            }
            return 1;
        }
        WM_TRAY => match (lp & 0xffff) as u32 {
            WM_LBUTTONUP | WM_LBUTTONDBLCLK => show_home(),
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
                    apply_theme(home(|m| is_dark(&m.settings)));
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
            if h != hwnd() { DestroyWindow(h); return 0; }
            let (tray, hinted) = home(|m| (m.settings.tray, m.settings.tray_hint_shown));
            if tray {
                ShowWindow(h, SW_HIDE);
                if !hinted {
                    let st = home(|m| {
                        m.settings.tray_hint_shown = true;
                        m.settings.clone()
                    });
                    store::save_settings(&st);
                    super::notify("Itda는 계속 실행 중입니다", "알림 영역 아이콘에서 다시 열거나 종료할 수 있습니다.");
                }
            } else {
                DestroyWindow(h);
            }
        }
        WM_DESTROY => {
            if h != hwnd() {
                let id = FILE_PANES.with(|p| p.borrow().iter().find(|(_, (wh, _))| *wh == h as isize).map(|(id, _)| id.clone()));
                if let Some(id) = id { FILE_HOSTS.with(|p| {p.borrow_mut().remove(&id);}); let removed = FILE_PANES.with(|p| p.borrow_mut().remove(&id)); drop(removed); super::send_hosts(); }
                return 0;
            }
            crate::files::shutdown();
            let wv = WEBVIEW.with(|w| w.borrow_mut().take());
            drop(wv);
            PostQuitMessage(0);
        }
        _ if m != 0 && m == TRAY_MSG.load(Relaxed) => tray_add(h),
        _ => return DefWindowProcW(h, m, wp, lp),
    }
    0
}
