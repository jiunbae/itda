//! Home window: the address book, settings and connection flow, rendered in a webview from
//! the embedded `web/` assets. The page logic here is shared; the window, tray/menu and
//! sessions are per platform (`home_win.rs`: WebView2, each connection in its own
//! `itda --session` process; `home_mac.rs`: WKWebView, each connection in a window over the
//! web viewer).

use crate::store::{self, Host, Settings};
use serde_json::{json, Value};
use std::borrow::Cow;
use std::cell::RefCell;
use std::collections::HashMap;
use std::net::{TcpStream, ToSocketAddrs};
use std::sync::atomic::Ordering::Relaxed;
use std::sync::Mutex;
use std::time::Duration;

#[cfg(windows)]
#[path = "home_win.rs"]
mod plat;
#[cfg(target_os = "macos")]
#[path = "home_mac.rs"]
mod plat;

const VERSION: &str = env!("CARGO_PKG_VERSION");

// Embedded UI. Set ITDA_WEB_DIR to serve them from disk while working on the UI.
const INDEX_HTML: &str = include_str!("../web/index.html");
const APP_CSS: &str = include_str!("../web/app.css");
const APP_JS: &str = include_str!("../web/app.js");
const ICONS_JS: &str = include_str!("../web/icons.js");
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
    /// A line from a session (process or window).
    Session(String, Value),
    /// A session ended.
    Exit(String),
    Online(HashMap<String, bool>),
    Found(Vec<crate::mdns::Found>),
    Tailnet(crate::tailscale::Status, bool),
    /// Result of turning `tailscale serve` on/off.
    TsServe(Result<(), String>),
    /// Someone started or stopped viewing this PC.
    #[cfg_attr(not(windows), allow(dead_code))]
    Share,
    /// An address-book change from the web viewer (a page command); answered when saved.
    WebEdit(Value, std::sync::mpsc::Sender<()>),
    /// A message for the page, produced off the UI thread.
    #[cfg_attr(windows, allow(dead_code))]
    Page(Value),
}

static QUEUE: Mutex<Vec<Event>> = Mutex::new(Vec::new());

fn post(e: Event) {
    QUEUE.lock().unwrap().push(e);
    plat::wake();
}

struct Sess {
    proc: plat::Proc,
    /// "connecting" | "connected" | "reconnecting"
    state: String,
    stage: String,
    /// Password to store once the connection is proven to work.
    save_pw: Option<String>,
}

#[derive(Default)]
struct Home {
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
}

fn home<R>(f: impl FnOnce(&mut Home) -> R) -> R {
    HOME.with(|h| f(&mut h.borrow_mut()))
}

fn is_dark(s: &Settings) -> bool {
    match s.theme.as_str() {
        "dark" => true,
        "light" => false,
        _ => plat::system_dark(),
    }
}

pub fn run(quick: Option<String>) {
    plat::run(quick);
}

/// Load the address book and settings (the platform calls this once it owns the window).
fn init(quick: Option<String>) {
    let settings = store::load_settings();
    let hosts = store::load_hosts();
    let accounts = store::load_accounts();
    home(|m| {
        m.settings = settings;
        m.hosts = hosts;
        m.accounts = accounts;
        m.pending_quick = quick;
    });
    // The web viewer's edits go through here so this list stays the one copy.
    let _ = crate::gateway::ON_EDIT.set(|v| {
        let (tx, rx) = std::sync::mpsc::channel();
        post(Event::WebEdit(v, tx));
        rx.recv_timeout(Duration::from_secs(5)).is_ok()
    });
}

/// The window is up: start the background work.
fn started() {
    plat::apply_zoom(home(|m| m.settings.zoom));
    start_probes();
    let w = home(|m| m.settings.web.clone());
    if w.on {
        apply_web(&w);
    }
    #[cfg(windows)]
    {
        let sh = home(|m| m.settings.share.clone());
        if sh.on {
            apply_share(&sh);
        }
    }
    scan();
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
        "icons.js" => ICONS_JS,
        "icon.svg" => ICON_SVG,
        "help.js" => HELP_JS,
        "help.css" => HELP_CSS,
        _ => return None,
    };
    Some((Cow::Borrowed(body.as_bytes()), mime))
}

/// Send a message to the page.
fn send(v: Value) {
    if !home(|m| m.ready) {
        return;
    }
    if test_bridge::active() {
        return test_bridge::push(v.to_string());
    }
    plat::eval(&format!("window.itda&&itda.recv({v})"));
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
            "platform": plat::PLATFORM,
            "computer": plat::computer_name(),
            "hosts": m.hosts.iter().map(|h| host_json(h, &m.accounts)).collect::<Vec<_>>(),
            "accounts": accounts_json(m),
            "settings": m.settings,
            "sessions": sessions_json(m),
            "online": m.online,
            "found": m.found,
            "dark": plat::system_dark(),
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

fn on_ipc(v: Value) {
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
        "pick_file" => plat::pick_file(s(&v, "field")),
        "forget_password" => {
            store::set_password(&id, None);
            send_hosts();
        }
        "settings" => {
            if let Ok(st) = serde_json::from_value::<Settings>(v["settings"].clone()) {
                let old = home(|m| std::mem::replace(&mut m.settings, st.clone()));
                store::save_settings(&st);
                plat::apply_theme(is_dark(&st));
                if old.zoom != st.zoom {
                    plat::apply_zoom(st.zoom);
                }
                send(json!({"type": "settings", "settings": st}));
                // Restart servers only when their own settings changed (restarts drop viewers).
                if old.web != st.web {
                    apply_web(&st.web);
                }
                #[cfg(windows)]
                if old.share != st.share {
                    apply_share(&st.share);
                }
            }
        }
        "web_info" => send(web_info()),
        "share_info" => send(share_info()),
        #[cfg(windows)]
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
        #[cfg(windows)]
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
                    let d = plat::screenshots_dir();
                    let _ = std::fs::create_dir_all(&d);
                    d
                }
                _ => return,
            };
            let _ = std::fs::create_dir_all(target.parent().unwrap_or(&target));
            plat::open(&target.to_string_lossy());
        }
        "url" => {
            let u = s(&v, "url");
            if u.starts_with("https://") || u.starts_with("x-apple") {
                plat::open(&u);
            }
        }
        "minimize" => plat::minimize(),
        c => crate::log!("ipc: unknown command {c}"),
    }
}

// ---------------------------------------------------------------- share this PC

#[cfg(windows)]
static SHARE_ERR: Mutex<String> = Mutex::new(String::new());
#[cfg(windows)]
static SHARE_SEEN: Mutex<Vec<String>> = Mutex::new(Vec::new());

#[cfg(windows)]
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

#[cfg(windows)]
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
        "name": plat::computer_name(),
        "error": *SHARE_ERR.lock().unwrap(),
    })
}

/// Elsewhere the system's own screen sharing is the way in (macOS: Screen Sharing).
#[cfg(not(windows))]
fn share_info() -> Value {
    let ts = TAILNET.lock().unwrap().clone().filter(|t| t.running).map(|t| t.me.dns).filter(|d| !d.is_empty());
    json!({"type": "share", "running": false, "builtin": true, "name": plat::computer_name(), "local_name": plat::local_name(), "user": std::env::var("USER").unwrap_or_default(), "tailscale": ts})
}

// ---------------------------------------------------------------- web viewer

static WEB_ERR: Mutex<String> = Mutex::new(String::new());
static TS_ERR: Mutex<String> = Mutex::new(String::new());
static TAILNET: Mutex<Option<crate::tailscale::Status>> = Mutex::new(None);
static SERVING: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Start/stop the gateway to match the settings, then tell the page.
fn apply_web(w: &store::Web) {
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

fn connect(id: &str, typed: Option<String>, save: bool) {
    if home(|m| m.sessions.contains_key(id)) {
        return focus_session(id);
    }
    let Some(host) = home(|m| m.hosts.iter().find(|x| x.id == id).cloned()) else { return };
    let login = home(|m| store::resolve(&host, &m.accounts));
    #[cfg(windows)]
    if host.is_rdp() {
        return match plat::open_rdp(&host, &login, typed) {
            Ok(()) => {
                home(|m| m.hosts.iter_mut().filter(|h| h.id == id).for_each(|h| h.last_used = store::now()));
                save_hosts();
                send_hosts();
                send(json!({"type": "toast", "text": format!("{} — 원격 데스크톱을 열었습니다", host.label())}));
            }
            Err(e) => send(json!({"type": "error", "id": id, "kind": "other", "msg": format!("원격 데스크톱(mstsc)을 열 수 없습니다: {e}")})),
        };
    }
    let pass = match typed.clone().or_else(|| login.pass.clone()) {
        Some(p) => p,
        None => {
            send(json!({"type": "need_password", "id": id}));
            return;
        }
    };
    let settings = home(|m| m.settings.clone());
    let proc = match plat::spawn_session(&host, &login, pass, &settings) {
        Ok(p) => p,
        Err(e) => {
            send(json!({"type": "error", "id": id, "kind": "other", "msg": format!("세션을 시작할 수 없습니다: {e}")}));
            return;
        }
    };
    let save_pw = if save { typed } else { None };
    home(|m| {
        m.sessions.insert(id.to_string(), Sess { proc, state: "connecting".into(), stage: "start".into(), save_pw });
    });
    send_session(id);
}

fn stop_session(id: &str) {
    home(|m| {
        if let Some(s) = m.sessions.get_mut(id) {
            plat::stop_session(&mut s.proc);
        }
    });
}

fn focus_session(id: &str) {
    home(|m| {
        if let Some(s) = m.sessions.get(id) {
            plat::focus_session(&s.proc);
        }
    });
}

fn host_label(id: &str) -> String {
    home(|m| m.hosts.iter().find(|x| x.id == id).map(|h| h.label().to_string())).unwrap_or_default()
}

/// Keep a session's last picture (a PNG data URL from the window) as the card thumbnail.
fn save_thumb(id: &str, url: &str) -> bool {
    let Some(b64) = url.strip_prefix("data:image/png;base64,") else { return false };
    if !home(|m| m.settings.thumbnails) || !id.chars().all(|c| c.is_ascii_hexdigit()) || b64.len() > 4 << 20 {
        return false;
    }
    match crate::ws::base64_decode(b64) {
        Some(png) if png.starts_with(b"\x89PNG") => std::fs::write(store::thumb_path(id), png).is_ok(),
        _ => false,
    }
}

fn on_session(id: &str, v: Value) {
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
            let label = host_label(id);
            let pw = home(|m| {
                let x = m.sessions.get_mut(id)?;
                x.state = "connected".into();
                plat::session_connected(&mut x.proc, &v, &label);
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
            if kind == "lost" || kind == "auth" {
                notify(&format!("{} 연결 종료", host_label(id)), &s(&v, "msg"));
            }
            plat::show_home();
            send(json!({"type": "error", "id": id, "kind": kind, "msg": s(&v, "msg")}));
        }
        "reconnecting" => {
            home(|m| {
                if let Some(x) = m.sessions.get_mut(id) {
                    x.state = "reconnecting".into();
                }
            });
            if v["n"].as_u64() == Some(1) {
                notify(&format!("{} 연결이 끊겼습니다", host_label(id)), "자동으로 다시 연결하는 중입니다.");
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
        "thumb" => {
            if v.get("png").is_none() || save_thumb(id, &s(&v, "png")) {
                send(json!({"type": "thumb", "id": id, "v": thumb_version(id)}));
            }
        }
        "closed" => {}
        // The connection's window asked to close (macOS).
        "close" => stop_session(id),
        e => crate::log!("session {id}: unknown event {e}"),
    }
}

fn on_exit(id: &str) {
    let gone = home(|m| m.sessions.remove(id));
    if let Some(s) = gone {
        let early = s.state == "connecting";
        if plat::reap(s.proc) && early {
            // Died before reporting anything useful.
            send(json!({"type": "error", "id": id, "kind": "other", "msg": "연결 프로세스가 예기치 않게 종료되었습니다"}));
        }
    }
    send_session(id);
}

fn drain() {
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
            #[cfg(windows)]
            Event::Share => {
                let now = crate::share::viewers();
                let fresh: Vec<String> = now.iter().map(|v| v.peer.clone()).filter(|p| !SHARE_SEEN.lock().unwrap().contains(p)).collect();
                *SHARE_SEEN.lock().unwrap() = now.iter().map(|v| v.peer.clone()).collect();
                for p in fresh {
                    notify("누군가 이 PC를 보고 있습니다", &format!("{p}에서 연결했습니다. 알림 영역의 Itda에서 끊을 수 있습니다."));
                }
                plat::tray_tip(now.len());
                send(share_info());
            }
            #[cfg(not(windows))]
            Event::Share => {}
            Event::TsServe(r) => {
                *TS_ERR.lock().unwrap() = r.err().unwrap_or_default();
                scan_tailnet();
                send(web_info());
            }
            Event::WebEdit(v, done) => {
                on_ipc(v);
                let _ = done.send(());
            }
            Event::Page(v) => send(v),
        }
    }
}

/// A notification from the system (when the settings allow).
fn notify(title: &str, text: &str) {
    if home(|m| m.settings.notify) {
        plat::notify(title, text);
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

/// Test-only stand-in for the webview (`ITDA_TEST_HTTP=<port>`): serves the same page on
/// 127.0.0.1 and carries the same IPC over HTTP, so a headless browser can drive the real
/// home logic where the webview can't run (e.g. a non-interactive window station).
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
                    super::post(super::Event::Session("__ipc".into(), v));
                }
                ("204 No Content", "text/plain", Vec::new())
            }
            // What a connection would report (macOS windows): drives the home's session logic.
            (true, p) if p.starts_with("/session/") => {
                let id = p.trim_start_matches("/session/").to_string();
                match serde_json::from_slice::<serde_json::Value>(&body) {
                    Ok(v) if v["ev"] == "exit" => super::post(super::Event::Exit(id)),
                    Ok(v) => super::post(super::Event::Session(id, v)),
                    Err(_) => {}
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
