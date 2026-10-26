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
const FILES_JS: &str = include_str!("../web/files.js");
const FILES_CSS: &str = include_str!("../web/files.css");
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
    ConfigurationStop,
    /// A waited-for (or stopped) set of transfers ended, or this many are still ending; with
    /// the ticket of the quit it belongs to.
    QuitDrained(QuitReason, usize, u64),
    /// A quit asked for from a window message that must not re-enter the page (Windows).
    #[cfg_attr(not(windows), allow(dead_code))]
    QuitAsk(QuitReason),
}

static QUEUE: Mutex<Vec<Event>> = Mutex::new(Vec::new());

fn post(mut e: Event) {
    if let Event::Page(v) = &mut e { crate::files::route_event(v); }
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
    /// The current in-app copy/cut, also sent to file windows opened after Copy.
    file_clipboard: Value,
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
    std::thread::spawn(|| loop {
        std::thread::sleep(std::time::Duration::from_millis(500));
        let path=store::dir().join("remote-setup-stop");
        if let Ok(value)=std::fs::read_to_string(&path) {
            let _=std::fs::remove_file(path);
            if value.trim().parse::<u64>().ok().is_some_and(|expires| expires>=store::now()&&expires<=store::now()+30) {
                post(Event::ConfigurationStop);
            }
        }
    });
    let mut settings = store::load_settings();
    settings.login_start = crate::autostart::state().on();
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
    // (Just started: checked often for a while, as after the home comes forward.)
    *HOME_LOOKED.lock().unwrap() = Some(std::time::Instant::now());
    std::thread::spawn(crate::files::sweep_view_copies);
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
    // A native UI fixture must not discover real peers or request local-network
    // permission. Require both an explicit opt-in and a separate data directory;
    // setting the test flag alone must not disable discovery in a normal profile.
    let fixture_without_discovery = store::override_base().is_some()
        && std::env::var("ITDA_NATIVE_TEST_NO_DISCOVERY").as_deref() == Ok("1");
    if !fixture_without_discovery {
        announce();
        start_discovery();
    } else {
        crate::log!("native fixture: automatic discovery disabled for isolated profile");
    }
    start_update_checks();
    // Again each launch: the entry names this exe, which may have moved. (Only while the entry
    // is there: one removed in Windows' settings stays removed. On macOS the item follows the app.)
    #[cfg(windows)]
    if !store::isolated() && crate::autostart::state().on() {
        if let Err(e) = crate::autostart::set(true) {
            crate::log!("login start: {e}");
        }
    }
}

/// A newer release found by the background check, offered to the page.
static UPDATE: Mutex<Option<crate::update::Release>> = Mutex::new(None);
/// Why the last update's swap failed (from the helper), told to the page once.
static UPDATE_FAILED: Mutex<Option<String>> = Mutex::new(None);
static UPDATING: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
fn update_json(r: &Option<crate::update::Release>) -> Value {
    r.as_ref().map_or(Value::Null, |r| json!({"version": r.version, "page": r.page}))
}
fn start_update_checks() {
    crate::update::cleanup();
    *UPDATE_FAILED.lock().unwrap() = crate::update::take_failure();
    std::thread::spawn(|| {
        std::thread::sleep(std::time::Duration::from_secs(20));
        loop {
            if home(|m| m.settings.auto_update) {
                if let Ok(Some(r)) = crate::update::check() {
                    let fresh = UPDATE.lock().unwrap().as_ref() != Some(&r);
                    *UPDATE.lock().unwrap() = Some(r.clone());
                    if fresh { post(Event::Page(json!({"type": "update", "available": update_json(&Some(r)), "found": true}))); }
                }
            }
            std::thread::sleep(std::time::Duration::from_secs(24 * 3600));
        }
    });
}
fn update_check() {
    std::thread::spawn(|| {
        let v = match crate::update::check() {
            Ok(r) => {
                *UPDATE.lock().unwrap() = r.clone();
                json!({"type": "update", "checked": true, "available": update_json(&r)})
            }
            Err(e) => json!({"type": "update", "checked": true, "error": e.to_string()}),
        };
        post(Event::Page(v));
    });
}
fn update_install() {
    if UPDATE.lock().unwrap().is_none() { return update_check() };
    // Installing replaces the app and restarts it: running transfers are asked about first,
    // before anything is downloaded or replaced.
    request_quit(QuitReason::Update);
}
fn start_update_install(r: crate::update::Release) {
    // go_now reserved the updater's closed admission gate under QUIT_PENDING.
    debug_assert!(UPDATING.load(std::sync::atomic::Ordering::SeqCst));
    std::thread::spawn(move || {
        let stage = |s: &str| post(Event::Page(json!({"type": "update", "stage": s})));
        match crate::update::install(&r, &stage) {
            Ok(()) => {
                crate::log!("update: installing {}, quitting", r.version);
                stage("다시 시작하는 중…");
                let ids: Vec<String> = home(|m| m.sessions.keys().cloned().collect());
                for id in ids { stop_session(&id); }
                std::thread::sleep(std::time::Duration::from_millis(400));
                {
                    let mut pending = QUIT_PENDING.lock().unwrap();
                    commit_quit_locked(&mut pending);
                }
                plat::quit();
            }
            Err(e) => {
                update_install_failed();
                post(Event::Page(json!({"type": "update", "error": e.to_string()})));
            }
        }
    });
}

fn asset(path: &str) -> Option<(Cow<'static, [u8]>, &'static str)> {
    let path = path.trim_start_matches('/');
    let path = if path.is_empty() { "index.html" } else { path };
    if let Some(id) = path.strip_prefix("thumb/").and_then(|p| p.strip_suffix(".png")) {
        if id.chars().all(|c| c.is_ascii_hexdigit()) {
            return std::fs::read(store::shown_thumb_path(id)).ok().map(|b| (Cow::Owned(b), "image/png"));
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
        "files.js" => FILES_JS,
        "files.css" => FILES_CSS,
        "icons.js" => ICONS_JS,
        "icon.svg" => ICON_SVG,
        "help.js" => HELP_JS,
        "help.css" => HELP_CSS,
        "hp-setup.css" => include_str!("../web/hp-setup.css"),
        "hp-setup.js" => include_str!("../web/hp-setup.js"),
        "remote-setup.js" => include_str!("../web/remote-setup.js"),
        "remote-setup.css" => include_str!("../web/remote-setup.css"),
        "permissions.html" => include_str!("../web/permissions.html"),
        "permissions.css" => include_str!("../web/permissions.css"),
        "permissions.js" => include_str!("../web/permissions.js"),
        "permission-guide.html" => include_str!("../web/permission-guide.html"),
        "permission-guide.css" => include_str!("../web/permission-guide.css"),
        "permission-guide.js" => include_str!("../web/permission-guide.js"),
        _ => return None,
    };
    Some((Cow::Borrowed(body.as_bytes()), mime))
}

pub(crate) fn binary_previews() -> bool { !test_bridge::active() }

/// Send a message to the page.
fn send(v: Value) {
    if !home(|m| m.ready) {
        return;
    }
    if test_bridge::active() {
        return test_bridge::push(v.to_string());
    }
    let script = format!("window.itda&&itda.recv({v})");
    if v["type"] == "files_result" {
        if let Some(owner) = crate::files::preview::result_owner(&v) {
            plat::eval_files(&owner, &script);
        }
    } else { plat::eval(&script); }
}

fn thumb_version(id: &str) -> u64 {
    std::fs::metadata(store::shown_thumb_path(id))
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
    v["custom_thumb"] = json!(store::custom_thumb_path(&h.id).is_file());
    v["rdp_key"] = json!(h.is_rdp() && store::rdp_key(&h.id).is_some());
    v
}

fn send_thumb(id: &str) {
    send(json!({"type": "thumb", "id": id, "v": thumb_version(id), "custom": store::custom_thumb_path(id).is_file()}));
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
    let v = home(|m| json!({"type": "hosts", "hosts": m.hosts.iter().map(|h| host_json(h, &m.accounts)).collect::<Vec<_>>(), "accounts": accounts_json(m), "file_windows": plat::file_windows()}));
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
            "login_item": crate::autostart::state().name(),
            "sessions": sessions_json(m),
            "file_windows": plat::file_windows(),
            "files_clipboard": m.file_clipboard,
            "files_jobs": crate::files::background_jobs(),
            "online": m.online,
            "found": m.found,
            "dark": plat::system_dark(),
            "ssh_ok": crate::tunnel::ssh_exe().is_some(),
            "tailnet": *TAILNET.lock().unwrap(),
            "quick": m.pending_quick.take(),
            "update": update_json(&UPDATE.lock().unwrap()),
            "update_failed": UPDATE_FAILED.lock().unwrap().take(),
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

// ---------------------------------------------------------------- quitting with transfers running

/// Why Itda is about to end: the person (menu, Dock, tray), an update that restarts it, or
/// the remote installer that was allowed to close the running app.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum QuitReason { User, Update, Configuration }
#[derive(PartialEq, Debug)]
enum QuitStep { Now, Ask, StopThenQuit }
/// Nothing running: go. The person is asked; an unattended installer stops them (it has a
/// short window and nobody to answer).
fn quit_step(reason: QuitReason, active: usize) -> QuitStep {
    if active == 0 { QuitStep::Now } else if reason == QuitReason::Configuration { QuitStep::StopThenQuit } else { QuitStep::Ask }
}
/// A quit (or update) waiting for transfers to end, or for stopped ones to clean up.
#[derive(Clone, Copy, PartialEq, Debug)]
enum QuitPending { Waiting(QuitReason), Stopping(QuitReason) }
/// The pending quit and its ticket: a wait or stop reports back with the ticket it was given,
/// and a report for one cancelled or replaced since is ignored.
static QUIT_PENDING: Mutex<Option<(u64, QuitPending)>> = Mutex::new(None);
/// A final quit has committed: no worker/cancel may reopen transfer admission.
static QUITTING: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
/// All callers hold QUIT_PENDING. These are the only production admission resumes.
fn resume_admissions_locked(pending: &Option<(u64, QuitPending)>) {
    if !UPDATING.load(std::sync::atomic::Ordering::SeqCst)
        && !QUITTING.load(std::sync::atomic::Ordering::SeqCst)
        && !matches!(pending, Some((_, QuitPending::Stopping(_)))) {
        crate::files::open_admissions();
    }
}
fn commit_quit_locked(pending: &mut Option<(u64, QuitPending)>) {
    crate::files::close_admissions();
    QUITTING.store(true, std::sync::atomic::Ordering::SeqCst);
    *pending = None;
}
fn update_install_failed() {
    let pending = QUIT_PENDING.lock().unwrap();
    UPDATING.store(false, std::sync::atomic::Ordering::SeqCst);
    resume_admissions_locked(&pending);
}
fn update_busy() {
    post(Event::Page(json!({"type":"toast","text":"업데이트를 적용하고 있어 Itda를 종료하거나 다시 열 수 없습니다. 완료될 때까지 기다려 주세요.","err":true})));
}
static QUIT_TICKETS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
fn pend(p: QuitPending) -> u64 {
    let t = QUIT_TICKETS.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    *QUIT_PENDING.lock().unwrap() = Some((t, p));
    t
}
const QUIT_STOP_WAIT: std::time::Duration = std::time::Duration::from_secs(15);
const QUIT_INSTALLER_WAIT: std::time::Duration = std::time::Duration::from_secs(8);

/// `request_quit` from the event queue (outside the window message being handled).
#[cfg_attr(not(windows), allow(dead_code))]
pub fn request_quit_later(reason: QuitReason) {
    post(Event::QuitAsk(reason));
}

/// Quit now if nothing runs; otherwise ask (or, for the installer, stop and wait). True
/// when Itda is ending now (the Mac quit Apple Event is answered "cancelled" otherwise).
pub fn request_quit(reason: QuitReason) -> bool {
    request_quit_with(reason, go_now)
}
fn request_quit_with(reason: QuitReason, go: impl FnOnce(QuitReason) -> bool) -> bool {
    if UPDATING.load(std::sync::atomic::Ordering::SeqCst) {
        update_busy();
        return false;
    }
    if QUITTING.load(std::sync::atomic::Ordering::SeqCst) { return true; }
    let n = crate::files::active_jobs();
    match quit_step(reason, n) {
        QuitStep::Now => go(reason),
        QuitStep::StopThenQuit => {
            crate::log!("quit: the remote installer closes Itda; stopping {n} transfer(s)");
            stop_then(reason, QUIT_INSTALLER_WAIT);
            false
        }
        QuitStep::Ask => { ask_quit(reason, n, false); false }
    }
}
/// The stuck question that may be answered with a forced quit (its ticket): only a plain
/// quit's, only the one shown last.
static FORCE_TICKET: Mutex<Option<u64>> = Mutex::new(None);
fn ask_quit(reason: QuitReason, count: usize, stuck: bool) {
    plat::show_home();
    let pending = *QUIT_PENDING.lock().unwrap();
    let force = (stuck && reason == QuitReason::User).then(|| QUIT_TICKETS.fetch_add(1, std::sync::atomic::Ordering::SeqCst));
    *FORCE_TICKET.lock().unwrap() = force;
    send(json!({"type": "quit_guard", "reason": reason_name(reason), "count": count, "stuck": stuck, "force": force, "waiting": matches!(pending, Some((_, QuitPending::Waiting(_))))}));
}
fn reason_name(r: QuitReason) -> &'static str {
    match r { QuitReason::User => "quit", QuitReason::Update => "update", QuitReason::Configuration => "configuration" }
}
/// Nothing may start from here on; if something did start since the count, ask instead.
fn go_now(reason: QuitReason) -> bool {
    let release = if reason == QuitReason::Update { UPDATE.lock().unwrap().clone() } else { None };
    go_now_target(reason, release, update_check, start_update_install, plat::quit)
}
fn go_now_target<T>(reason: QuitReason, release: Option<T>, check: impl FnOnce(), install: impl FnOnce(T), quit: impl FnOnce()) -> bool {
    if reason == QuitReason::Update && release.is_none() {
        // The version can disappear while a transfer wait is draining. Do not
        // reserve UPDATING without a concrete release or strand its Cancel bar.
        let cancelled = {
            let mut pending = QUIT_PENDING.lock().unwrap();
            if UPDATING.load(std::sync::atomic::Ordering::SeqCst) { drop(pending); update_busy(); return false; }
            if QUITTING.load(std::sync::atomic::Ordering::SeqCst) { return true; }
            let own_wait = matches!(*pending, Some((_, QuitPending::Waiting(QuitReason::Update))));
            let own_stop = matches!(*pending, Some((_, QuitPending::Stopping(QuitReason::Update))));
            if own_wait || own_stop {
                let was_open = crate::files::accepting();
                *pending = None;
                if was_open || own_stop { resume_admissions_locked(&pending); }
            }
            own_wait || own_stop
        };
        if cancelled { post(Event::Page(json!({"type":"quit_pending","count":0}))); }
        check();
        return false;
    }
    go_now_with(reason, || { if let Some(release) = release { install(release); } }, quit)
}
fn go_now_with(reason: QuitReason, install: impl FnOnce(), quit: impl FnOnce()) -> bool {
    let count = {
        let mut pending = QUIT_PENDING.lock().unwrap();
        if UPDATING.load(std::sync::atomic::Ordering::SeqCst) { drop(pending); update_busy(); return false; }
        if QUITTING.load(std::sync::atomic::Ordering::SeqCst) { return true; }
        let count = crate::files::close_admissions();
        if count > 0 {
            resume_admissions_locked(&pending);
        } else {
            *pending = None;
            if reason == QuitReason::Update {
                UPDATING.store(true, std::sync::atomic::Ordering::SeqCst);
            } else {
                QUITTING.store(true, std::sync::atomic::Ordering::SeqCst);
            }
        }
        count
    };
    if count > 0 {
        if reason == QuitReason::Configuration { stop_then(reason, QUIT_INSTALLER_WAIT) } else { ask_quit(reason, count, false) }
        return false;
    }
    // Consuming a waiting intent must also remove its now-stale Cancel button.
    post(Event::Page(json!({"type":"quit_pending","count":0})));
    if reason == QuitReason::Update { install(); } else { quit(); }
    true
}
/// Permission relaunch is never a stop/force choice. Called only on the UI thread.
/// Keep quit intent locked while reserving idle admission and launching the helper;
/// the updater's worker rollback takes the same lock before reopening admission.
#[cfg(target_os = "macos")]
pub(crate) fn permission_restart(launch: impl FnOnce() -> Result<(), String>) -> Result<(), String> {
    permission_restart_with(launch, plat::quit)
}
#[cfg(target_os = "macos")]
pub(crate) fn permission_restart_closing() -> bool {
    QUIT_PENDING.lock().unwrap().is_some() || QUITTING.load(std::sync::atomic::Ordering::SeqCst) || UPDATING.load(std::sync::atomic::Ordering::SeqCst) || !crate::files::accepting()
}
#[cfg(any(target_os = "macos", test))]
fn permission_restart_with(launch: impl FnOnce() -> Result<(), String>, quit: impl FnOnce()) -> Result<(), String> {
    let mut pending = QUIT_PENDING.lock().unwrap();
    if pending.is_some() || QUITTING.load(std::sync::atomic::Ordering::SeqCst) || UPDATING.load(std::sync::atomic::Ordering::SeqCst) || !crate::files::accepting() {
        return Err("Itda의 종료 또는 업데이트가 진행 중입니다. 끝난 뒤 권한 적용을 다시 시도해 주세요.".into());
    }
    // Job admission and this close+recount share the registry lock. A transfer
    // accepted after an earlier UI count is included; later attempts are refused.
    let count = crate::files::close_admissions();
    if count > 0 {
        resume_admissions_locked(&pending);
        return Err(format!("파일 전송 {count}개가 진행 중이라 Itda를 다시 열지 않았습니다. 전송이 끝난 뒤 다시 시도해 주세요."));
    }
    if let Err(error) = launch() {
        // Only this guard's open→closed reservation is rolled back. A prior
        // closed gate or quit intent was rejected above and remains untouched.
        resume_admissions_locked(&pending);
        return Err(error);
    }
    commit_quit_locked(&mut pending);
    drop(pending);
    quit();
    Ok(())
}

/// The person's answer: wait for the transfers, stop them, force the quit (only after a stop
/// that didn't finish, and never for an update), or keep Itda open.
fn quit_choice(choice: &str, reason: QuitReason, ticket: u64) {
    if QUITTING.load(std::sync::atomic::Ordering::SeqCst) { return; }
    if UPDATING.load(std::sync::atomic::Ordering::SeqCst) && choice != "cancel" { update_busy(); return; }
    if choice != "force" { *FORCE_TICKET.lock().unwrap() = None; }
    match choice {
        "wait" => {
            let ticket = pend(QuitPending::Waiting(reason));
            std::thread::spawn(move || watch_quit(ticket));
            post(Event::Page(json!({"type": "quit_pending", "reason": reason_name(reason), "count": crate::files::active_jobs()})));
        }
        "stop" => stop_then(reason, QUIT_STOP_WAIT),
        "force" if reason == QuitReason::User && FORCE_TICKET.lock().unwrap().take().is_some_and(|t| t == ticket) => {
            crate::log!("quit: forced with {} transfer(s) still ending", crate::files::active_jobs());
            {
                let mut pending = QUIT_PENDING.lock().unwrap();
                commit_quit_locked(&mut pending);
            }
            plat::quit();
        }
        _ => {
            let (stopping, cancelled) = {
                let mut pending = QUIT_PENDING.lock().unwrap();
                let stopping = matches!(*pending, Some((_, QuitPending::Stopping(_))));
                let cancelled = pending.is_some();
                *pending = None;
                resume_admissions_locked(&pending);
                (stopping, cancelled)
            };
            post(Event::Page(json!({"type": "quit_pending", "count": 0, "cancelled": cancelled, "stopped": stopping})));
        }
    }
}
/// Until nothing runs (transfers started meanwhile too), or this wait is cancelled/replaced.
fn watch_quit(ticket: u64) {
    loop {
        std::thread::sleep(std::time::Duration::from_millis(500));
        let Some((t, QuitPending::Waiting(reason))) = *QUIT_PENDING.lock().unwrap() else { return };
        if t != ticket { return }
        let n = crate::files::active_jobs();
        if n == 0 { return post(Event::QuitDrained(reason, 0, ticket)); }
        post(Event::Page(json!({"type": "quit_pending", "reason": reason_name(reason), "count": n})));
    }
}
/// Stop every transfer the normal way (nothing new starts meanwhile) and wait, bounded, for
/// them to clean up. What comes next is decided when they did (or didn't).
fn stop_then(reason: QuitReason, limit: std::time::Duration) {
    let ticket = {
        let mut pending = QUIT_PENDING.lock().unwrap();
        if UPDATING.load(std::sync::atomic::Ordering::SeqCst) { drop(pending); update_busy(); return; }
        if QUITTING.load(std::sync::atomic::Ordering::SeqCst) { return; }
        let ticket = QUIT_TICKETS.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        *pending = Some((ticket, QuitPending::Stopping(reason)));
        crate::files::close_admissions();
        ticket
    };
    crate::files::stop_jobs();
    std::thread::spawn(move || {
        let left = drain_jobs(limit, Some((reason, ticket)));
        post(Event::QuitDrained(reason, left, ticket));
    });
}
/// After a wait or a stop: go if everything ended. Otherwise Itda stays open and says so:
/// the person can wait again or (for a plain quit) force it; the installer is refused (its
/// own bounded wait then fails, and a busy app is never replaced).
// (`_reported`: what the waiting thread saw; the decision recounts.)
fn quit_drained(reason: QuitReason, _reported: usize, ticket: u64) {
    // Only the report of the quit still pending, for the same intent (not one cancelled or
    // replaced since); and the count is checked again.
    let current = matches!(*QUIT_PENDING.lock().unwrap(), Some((t, p)) if t == ticket && matches!(p, QuitPending::Waiting(r) | QuitPending::Stopping(r) if r == reason));
    if !current { return }
    // What runs now decides (a transfer may have ended, or started, since the report).
    let left = crate::files::active_jobs();
    if left == 0 { go_now(reason); return; }
    // Waiting, and a transfer started right then: keep waiting.
    if matches!(*QUIT_PENDING.lock().unwrap(), Some((_, QuitPending::Waiting(_)))) {
        std::thread::spawn(move || watch_quit(ticket));
        return;
    }
    {
        let mut pending = QUIT_PENDING.lock().unwrap();
        *pending = None;
        resume_admissions_locked(&pending);
    }
    send(json!({"type": "quit_pending", "count": 0}));
    crate::log!("quit: {left} transfer(s) did not end in time; Itda stays open");
    if reason == QuitReason::Configuration {
        notify("원격 설치를 위해 Itda를 닫지 못했습니다", &format!("전송 {left}개가 아직 정리 중입니다. 끝난 뒤 원격 설치를 다시 시도하세요."));
    } else {
        ask_quit(reason, left, true);
    }
}
/// Waits up to `limit` for running transfers to end; how many are left. With a stop's
/// reason and ticket, the main window is told what is still being cleaned up.
fn drain_jobs(limit: std::time::Duration, stopping: Option<(QuitReason, u64)>) -> usize {
    let until = std::time::Instant::now() + limit;
    let mut shown = usize::MAX;
    loop {
        let n = crate::files::active_jobs();
        if n == 0 || std::time::Instant::now() >= until { return n; }
        if let Some((reason, ticket)) = stopping {
            let current = matches!(*QUIT_PENDING.lock().unwrap(), Some((t, _)) if t == ticket);
            if !current { return n; }
            if n != shown {
                shown = n;
                post(Event::Page(json!({"type": "quit_pending", "reason": reason_name(reason), "count": n, "stopping": true})));
            }
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
}

/// Stopping or dismissing a transfer needs no device or login: the device may be the
/// device-less window's, or deleted since. Who may stop what is decided by files::dispatch
/// (a page its own transfers; any page those whose window closed).
fn files_control(v: &Value, id: &str, emit: crate::files::Emit) -> bool {
    if !["cancel", "cancel_background", "dismiss_background"].contains(&v["op"].as_str().unwrap_or("")) { return false; }
    crate::files::dispatch(v.clone(), Host { id: id.to_string(), ..Default::default() }, store::Login::default(), emit);
    true
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
        #[cfg(target_os = "macos")]
        "files_native_copy" => plat::native_file_copy(v),
        #[cfg(target_os = "macos")]
        "files_native_paste" => plat::native_file_paste(v),
        #[cfg(windows)]
        "files_native_drag" => plat::native_file_drag(v),
        "files_window" => {
            if v["close"] == true { plat::files_window(v["window"].as_str().unwrap_or(&id), true); }
            else if id == crate::files::locations::NETWORK || home(|m| m.hosts.iter().any(|h| h.id == id)) {
                if v["new"] == true { plat::files_new_window_at(&id, v["path"].as_str()); } else { plat::files_window(&id, false); }
            }
        }
        "files_window_context" => {
            if id == crate::files::locations::NETWORK || home(|m| m.hosts.iter().any(|h| h.id == id)) {
                plat::files_window_context(s(&v, "window").as_str(), &id, s(&v, "page").as_str());
            }
        }
        #[cfg(any(target_os = "macos", windows))]
        "files_drag_release" => { if let Some(token) = v["drag"].as_str() { crate::files::drag::release(token); } },
        "files_release_page" => crate::files::release_page(s(&v, "page").as_str()),
        // A sort chosen in one file window applies in every open one.
        "files_sort" => send(json!({"type": "files_sort", "sort": v["sort"]})),
        #[cfg(windows)]
        "file_window_control" => plat::file_window_control(s(&v, "window").as_str(), s(&v, "op").as_str()),
        // Copy/Cut in one file window is what Paste uses in every other one.
        "files_clipboard" => {
            let clip = v["clip"].clone();
            home(|m| {
                if let Some(clear) = clip["clear"].as_str() {
                    if m.file_clipboard["cid"].as_str() == Some(clear) { m.file_clipboard = Value::Null; }
                } else if clip["names"].as_array().is_some_and(|names| !names.is_empty() && names.len() <= 10_000) {
                    m.file_clipboard = clip.clone();
                }
                crate::files::remember_clipboard_source(&m.file_clipboard);
            });
            send(json!({"type": "files_clipboard", "clip": clip}));
        },
        "file_locations" => {
            let hosts = home(|m| m.hosts.clone());
            let reveal = v["reveal"] != false;
            std::thread::spawn(move || {
                let root = crate::files::locations::folder();
                let result = crate::files::locations::write(&hosts, &root);
                if result.is_ok() && reveal { plat::open(&root.to_string_lossy()); }
                post(Event::Page(json!({"type":"file_locations_result","ok":result.is_ok(),"path":root,"error":result.err().map(|e| e.to_string())})));
            });
        }
        "files" => {
            if files_control(&v, &id, std::sync::Arc::new(|v| post(Event::Page(v)))) { return; }
            // The device-less file window still browses this computer's own files.
            // Transfers from it land on this computer (their source may be another window's
            // device, named by from_id); they never write to a device through this window.
            let local_only = id == crate::files::locations::NETWORK
                && (v["op"] == "local_home"
                    || (v["side"] == "local" && v["op"] != "transfer")
                    || (v["op"] == "transfer" && v["to"] == "local" && (v["from"] == "local" || v["from_id"].as_str().is_some_and(|f| !f.is_empty() && f != id))));
            let found = home(|m| m.hosts.iter().find(|h| h.id == id).cloned());
            let Some(mut host) = found.or_else(|| local_only.then(|| Host { id: id.clone(), ..Default::default() })) else { return };
            if v["op"] == "finder" {
                std::thread::spawn(move || {
                    let result = crate::files::native_target(&host).and_then(|target| crate::files::open_native(&target));
                    post(Event::Page(json!({"type":"files_result","op":"finder","id":id,"window":v["window"],"ok":result.is_ok(),"error":result.err().map(|e|e.to_string())})));
                });
                return;
            }
            if v["op"] == "connect" {
                let options = match serde_json::from_value::<crate::files::Options>(v["options"].clone()) {
                    Ok(o) if ["auto", "sftp", "smb"].contains(&o.method.as_str()) => o,
                    _ => { send(json!({"type":"files_result","id":id,"window":v["window"],"op":"connect","ok":false,"error":"파일 연결 설정을 확인하세요"})); return; }
                };
                {
                    host.files = options;
                    home(|m| { if let Some(h) = m.hosts.iter_mut().find(|h| h.id == id) { h.files = host.files.clone(); } });
                    save_hosts();
                    send_hosts();
                }
                if host.files.method == "smb" || (host.files.method == "auto" && !host.files.share.is_empty()) {
                    std::thread::spawn(move || {
                        let result = crate::files::native_target(&host).and_then(|target| crate::files::open_native(&target));
                        post(Event::Page(json!({"type":"files_result","op":"native","id":id,"window":v["window"],"ok":result.is_ok(),"error":result.err().map(|e|e.to_string())})));
                    });
                    return;
                }
            }
            let login = home(|m| {
                if host.files.account == "custom" {
                    Some(store::Login { user: host.files.user.clone(), ..Default::default() })
                } else if host.files.account.is_empty() {
                    Some(store::resolve(&host, &m.accounts))
                } else if m.accounts.iter().any(|a| a.id == host.files.account) {
                    let mut selected = host.clone();
                    selected.account = host.files.account.clone();
                    Some(store::resolve(&selected, &m.accounts))
                } else { None }
            });
            let Some(login) = login else { send(json!({"type":"files_result","id":id,"window":v["window"],"op":"connect","ok":false,"error":"저장 계정이 삭제되었습니다. 사용할 계정을 다시 선택하세요"})); return; };
            crate::files::dispatch(v, host, login, std::sync::Arc::new(|v| post(Event::Page(v))));
        }
        "cancel" | "disconnect" => stop_session(&id),
        "focus" => focus_session(&id),
        "save_host" => {
            let Ok(mut host) = serde_json::from_value::<Host>(v["host"].clone()) else { return };
            host.host = host.host.trim().to_string();
            host.name = host.name.trim().to_string();
            host.user = host.user.trim().to_string();
            host.normalize_groups();
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
                    // Another device now: the Remote Desktop key seen before isn't its.
                    if x.host.trim().to_lowercase() != host.host.trim().to_lowercase() || x.kind != host.kind {
                        store::forget_rdp_key(&id);
                    }
                    // Keep fields the editor doesn't own.
                    host.files = x.files.clone();
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
            crate::files::disconnect(&id);
            // A copy from this device can't be pasted any more: windows drop it.
            let cid = home(|m| (m.file_clipboard["host"].as_str() == Some(id.as_str())).then(|| std::mem::take(&mut m.file_clipboard)["cid"].as_str().map(str::to_string)).flatten());
            if let Some(cid) = cid {
                send(json!({"type": "files_clipboard", "clip": {"clear": cid}}));
            }
            if let Some(h) = home(|m| m.hosts.iter().find(|h| h.id == id).cloned()) { store::set_password(&crate::files::password_key(&h), None); }
            store::set_password(&format!("files-{id}"), None);
            home(|m| m.hosts.retain(|x| x.id != id));
            store::set_password(&id, None);
            store::remove_thumbs(&id);
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
        // A picture of the user's choosing for the card (a PNG data URL), or none: back to
        // the last screen.
        "set_thumb" => {
            if !home(|m| m.hosts.iter().any(|h| h.id == id)) {
                return;
            }
            let ok = match v["png"].as_str() {
                Some(url) => thumb_png(&id, url).is_some_and(|png| std::fs::write(store::custom_thumb_path(&id), png).is_ok()),
                None => std::fs::remove_file(store::custom_thumb_path(&id)).is_ok(),
            };
            if !ok {
                send(json!({"type": "toast", "text": "미리보기 이미지를 저장하지 못했습니다", "err": true}));
            }
            send_thumb(&id);
        }
        "forget_password" => {
            store::set_password(&id, None);
            send_hosts();
        }
        "settings" => {
            if let Ok(mut st) = serde_json::from_value::<Settings>(v["settings"].clone()) {
                // The system holds this one: keep what it actually did.
                if st.login_start != home(|m| m.settings.login_start) && store::isolated() {
                    st.login_start = home(|m| m.settings.login_start);
                    send(json!({"type":"toast","text":"별도 설정으로 실행한 앱에서는 로그인 시 실행을 변경할 수 없습니다","err":true}));
                } else if st.login_start != home(|m| m.settings.login_start) {
                    let (login, err) = crate::autostart::apply(st.login_start, crate::autostart::set, crate::autostart::state);
                    st.login_start = login.on();
                    if let Some(e) = err {
                        crate::log!("login start: {e}");
                        send(json!({"type": "toast", "text": format!("로그인 시 실행을 바꾸지 못했습니다: {e}"), "err": true}));
                    }
                    send(json!({"type": "login_item", "state": login.name()}));
                }
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
                if (old.announce, &old.web, &old.share) != (st.announce, &st.web, &st.share) {
                    announce();
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
        // "고성능 모드": set up Itda's own sharing on this Mac over SSH; the next connection
        // uses it. Progress and the outcome go to the pages as {type:"hp"}.
        "hp_upgrade" => hp_upgrade(&id),
        "update_check" => update_check(),
        "forget_rdp_key" => {
            store::forget_rdp_key(&id);
            send_hosts();
            send(json!({"type": "toast", "text": "원격 데스크톱 서버 키를 지웠습니다. 다음 연결에서 새로 받습니다"}));
        }
        // Settings opened: the login item may have been changed in the system's settings.
        "login_item" => {
            let s = crate::autostart::state();
            home(|m| m.settings.login_start = s.on());
            send(json!({"type": "login_item", "state": s.name()}));
        }
        "login_item_settings" => crate::autostart::open_settings(),
        "update_install" => update_install(),
        "quit_choice" => quit_choice(&s(&v, "choice"), match s(&v, "reason").as_str() { "update" => QuitReason::Update, _ => QuitReason::User }, v["ticket"].as_u64().unwrap_or(0)),
        "remote_setup_probe"=>remote_setup_probe(v),
        "remote_setup_apply"=>{
            let request=s(&v,"request");let token=s(&v,"token");
            match crate::remote_setup::begin() {
                Err(e)=>send(json!({"type":"remote_setup","request":request,"error":e.to_string()})),
                Ok(busy)=>{std::thread::spawn(move||{
                    let _busy=busy;
                    let progress=|stage:&str|post(Event::Page(json!({"type":"remote_setup","request":request,"stage":stage})));
                    let result=crate::remote_setup::install(&token,&progress);
                    let mut v=match result {Ok(v)=>v,Err(e)=>json!({"type":"remote_setup","error":e.to_string()})};
                    v["request"]=json!(request);post(Event::Page(v));
                });}
            }
        }
        "remote_setup_discard"=>crate::remote_setup::discard(&s(&v,"token")),
        "hp_status" => hp_status(&id, s(&v, "req")),
        "hp_permissions" => hp_permissions(&id,v["permission"].as_str().map(str::to_string)),
        #[cfg(target_os = "macos")]
        "permission_helper" => plat::permission_helper(None),
        "hp_ready" | "hp_off" => {
            let on = s(&v, "cmd") == "hp_ready";
            let port = v["port"].as_u64().unwrap_or(0) as u16;
            let changed = home(|m| m.hosts.iter_mut().find(|h| h.id == id).map(|h| h.hp = store::Hp { on, port: if on { port } else { h.hp.port }, native: h.hp.native }).is_some());
            if changed {
                save_hosts();
                send_hosts();
            }
        }
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
            trust_near: s.trust_near,
            cmd_as_ctrl: s.cmd_as_ctrl,
            view_only: s.view_only,
            local_only: false,
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

/// A PNG data URL from the page, for a card thumbnail of the connection `id`.
fn thumb_png(id: &str, url: &str) -> Option<Vec<u8>> {
    let b64 = url.strip_prefix("data:image/png;base64,")?;
    if !id.chars().all(|c| c.is_ascii_hexdigit()) || b64.len() > 4 << 20 {
        return None;
    }
    crate::ws::base64_decode(b64).filter(|png| png.starts_with(b"\x89PNG"))
}

/// Keep a session's last picture (a PNG data URL from the window) as the card thumbnail.
fn save_thumb(id: &str, url: &str) -> bool {
    home(|m| m.settings.thumbnails) && thumb_png(id, url).is_some_and(|png| std::fs::write(store::thumb_path(id), png).is_ok())
}

/// "고성능 모드" for host `id`: set up Itda's own sharing on that Mac over SSH (a worker
/// thread); progress and the outcome go to the pages and the connection's window as
/// {type:"hp"}, and the host remembers it (hp_ready).
fn hp_upgrade(id: &str) {
    let Some((host, login)) = home(|m| m.hosts.iter().find(|h| h.id == id).map(|h| (h.clone(), store::resolve(h, &m.accounts)))) else { return };
    std::thread::spawn(move || {
        use crate::hp_upgrade::{password_key, upgrade, Source};
        let id = host.id.clone();
        let source = std::env::var("ITDA_HP_BUNDLE").map(|p| Source::Bundle(p.into())).unwrap_or_else(|_| Source::Release(VERSION.into()));
        let progress = |stage: &str| post(Event::Page(json!({"type":"hp","id":id,"stage":stage})));
        // A password typed for this connection signs in to SSH first (a saved one may be stale).
        let pass = crate::gateway::typed(&id).or_else(|| login.pass.clone()).unwrap_or_default();
        match upgrade(&crate::hp_upgrade::ssh_cfg_with(&host, &login, pass), &source, &progress) {
            Ok(r) => {
                store::set_password(&password_key(&id), Some(&r.password));
                post(Event::Session("__ipc".into(), json!({"cmd":"hp_ready","id":id,"port":r.port})));
                post(Event::Page(json!({"type":"hp","id":id,"ready":true,"screen":r.screen,"access":r.access,"installed":r.installed,"helper_error":r.helper_error})));
            }
            Err(e) => post(Event::Page(json!({"type":"hp","id":id,"error":e.to_string()}))),
        }
    });
}

fn remote_setup_probe(v:Value) {
    let request=s(&v,"request");
    let result=(|| {
        let choice=serde_json::from_value::<crate::remote_setup::Selection>(v["selection"].clone())?;
        let (hosts,accounts,settings)=home(|m|(m.hosts.clone(),m.accounts.clone(),m.settings.clone()));
        let h=hosts.iter().find(|h|h.id==s(&v,"id"));
        let account=s(&v,"login_account");
        let login=if account=="-" {store::Login::default()}
        else if !account.is_empty() {
            let a=accounts.iter().find(|a|a.id==account).ok_or_else(||std::io::Error::other("SSH에 사용할 계정을 다시 선택하세요"))?;
            store::Login{user:a.user.clone(),pass:store::get_password(&store::acct_key(&a.id)),ssh_key:a.ssh_key.clone(),key_pass:store::get_password(&store::acct_keypass(&a.id))}
        } else {h.map(|h|store::resolve(h,&accounts)).unwrap_or_default()};
        let host=s(&v,"address");
        let port=v["port"].as_u64().filter(|p|*p>0&&*p<=65535).ok_or_else(||std::io::Error::other("SSH 포트는 1부터 65535 사이여야 합니다"))? as u16;
        let pass=match v["password"].as_str().filter(|p|!p.is_empty()){Some(p)=>p.into(),None=>if account.is_empty(){h.and_then(|h|crate::gateway::typed(&h.id)).or(login.pass).unwrap_or_default()}else{login.pass.unwrap_or_default()}};
        let cfg=crate::tunnel::SshCfg{host,port,user:s(&v,"user"),pass,vnc_port:0,key:login.ssh_key,key_pass:login.key_pass.unwrap_or_default(),key_alias:String::new()};
        crate::tunnel::ssh_target(&cfg)?;
        if cfg.host.trim().is_empty(){return Err(std::io::Error::other("설치할 기기의 SSH 주소를 입력하세요"));}
        let busy=crate::remote_setup::begin()?;
        Ok::<_,std::io::Error>((cfg,choice,hosts,accounts,settings,busy))
    })();
    match result {
        Err(e)=>send(json!({"type":"remote_setup","request":request,"error":e.to_string()})),
        Ok((cfg,choice,hosts,accounts,settings,busy))=>{std::thread::spawn(move|| {
            let _busy=busy;let progress=|stage:&str|post(Event::Page(json!({"type":"remote_setup","request":request,"stage":stage})));
            let mut result=match crate::remote_setup::prepare(cfg,choice,hosts,accounts,settings,&progress){Ok(v)=>v,Err(e)=>json!({"type":"remote_setup","error":e.to_string()})};
            result["request"]=json!(request);post(Event::Page(result));
        });}
    }
}

/// Whether that Mac's Itda can show its screen yet (the viewer waits for the permission).
/// `req`: the viewer's token for this request, echoed so it can drop a late answer.
fn hp_status(id: &str, req: String) {
    let Some((host, login)) = home(|m| m.hosts.iter().find(|h| h.id == id).map(|h| (h.clone(), store::resolve(h, &m.accounts)))) else { return };
    std::thread::spawn(move || {
        let pass = crate::gateway::typed(&host.id).or_else(|| login.pass.clone()).unwrap_or_default();
        let r = crate::hp_upgrade::status(&crate::hp_upgrade::ssh_cfg_with(&host, &login, pass));
        post(Event::Page(match r {
            Ok(status) => json!({"type":"hp","id":host.id,"req":req,"status":true,"screen":status.screen,"access":status.access,"running":status.running,"quality":status.quality,"quality_active":status.quality_active,"capture_width":status.capture.0,"capture_height":status.capture.1,"actions":status.actions}),
            Err(e) => json!({"type":"hp","id":host.id,"req":req,"status":true,"error":e.to_string()}),
        }));
    });
}

fn hp_permissions(id: &str, permission: Option<String>) {
    let Some((host,login))=home(|m|m.hosts.iter().find(|h|h.id==id).map(|h|(h.clone(),store::resolve(h,&m.accounts)))) else{return};
    std::thread::spawn(move||{
        let pass=crate::gateway::typed(&host.id).or_else(||login.pass.clone()).unwrap_or_default();
        let result=crate::hp_upgrade::show_permissions(&crate::hp_upgrade::ssh_cfg_with(&host,&login,pass),permission.as_deref());
        post(Event::Page(json!({"type":"hp","id":host.id,"helper":true,"error":result.err().map(|e|e.to_string())})));
    });
}

/// Sharper (native pixels) or faster (points) high-performance picture on that Mac: its Itda
/// reads the choice when the capture next starts, so the viewer reconnects after.
fn hp_quality(id: &str, native: bool) {
    let Some((host, login)) = home(|m| m.hosts.iter().find(|h| h.id == id).map(|h| (h.clone(), store::resolve(h, &m.accounts)))) else { return };
    std::thread::spawn(move || {
        let pass = crate::gateway::typed(&host.id).or_else(|| login.pass.clone()).unwrap_or_default();
        let r = crate::hp_upgrade::set_quality(&crate::hp_upgrade::ssh_cfg_with(&host, &login, pass), native);
        post(Event::Page(json!({"type": "hp", "id": host.id, "quality": native, "ok": r.is_ok(), "error": r.err().map(|e| e.to_string())})));
    });
}

fn on_session(id: &str, v: Value) {
    match s(&v, "ev").as_str() {
        "hp-upgrade" => hp_upgrade(id),
        "hp-status" => hp_status(id, v["req"].as_str().unwrap_or("").to_string()),
        "hp-permissions" => hp_permissions(id,v["permission"].as_str().map(str::to_string)),
        "hp-quality" => hp_quality(id, v["native"] == true),
        "hp-off" => on_ipc(json!({"cmd":"hp_off","id":id})),
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
                send_thumb(id);
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
            Event::ConfigurationStop=>{ request_quit(QuitReason::Configuration); }
            Event::QuitDrained(reason, left, ticket) => quit_drained(reason, left, ticket),
            Event::QuitAsk(reason) => { request_quit(reason); }
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
                if home(|m| learn_lan(&mut m.hosts, &st.peers)) {
                    save_hosts();
                    send_hosts();
                }
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
            Event::WebEdit(mut v, done) => {
                // Provisioning can export credentials; it is native-app-only, not
                // an address-book operation exposed by the web viewer.
                if !v["cmd"].as_str().is_some_and(|cmd|cmd.starts_with("remote_setup_")) {
                    if v["cmd"] == "save_host" {
                        let wipe = home(|m| web_save_guard(&mut v["host"], &m.hosts));
                        if let Some(id) = wipe {
                            forget_saved_logins(&id);
                            // Files follow the new address unless they have their own: then their
                            // account mustn't go along either. (save_host keeps `files` as stored.)
                            home(|m| m.hosts.iter_mut().filter(|x| x.id == id && x.files.host.trim().is_empty()).for_each(|x| x.files.account.clear()));
                        }
                    }
                    on_ipc(v);
                }
                let _ = done.send(());
            }
            Event::Page(v) => {
                if v["type"] == "hp" {
                    // A quality change that reached the Mac is this device's choice from now on.
                    if v["ok"] == true && v["quality"].is_boolean() {
                        let (id, native) = (v["id"].as_str().unwrap_or("").to_string(), v["quality"] == true);
                        if home(|m| m.hosts.iter_mut().find(|h| h.id == id).map(|h| h.hp.native = native).is_some()) {
                            save_hosts();
                            send_hosts();
                        }
                    }
                    if let Some(id) = v["id"].as_str() {
                        plat::session_eval(id, &format!("window.itdaHp&&itdaHp({v})"));
                    }
                }
                // A transfer whose file window closed: say how it ended, since no window shows it
                // (the main window says it itself when it is in front).
                if v["type"] == "files_job" && v["window"] == "app" && v["dismissed"] != true && !plat::home_focused()
                    && ["completed", "failed", "cancelled"].contains(&v["state"].as_str().unwrap_or("")) {
                    let what = if v["moving"] == true || v["request"]["move"] == true { "파일 이동" } else { "파일 복사" };
                    let device = Some(host_label(v["id"].as_str().unwrap_or(""))).filter(|d| !d.is_empty()).unwrap_or_else(|| "이 컴퓨터".into());
                    let text = match v["state"].as_str() {
                        Some("completed") => format!("{device}: 끝났습니다"),
                        Some("cancelled") => format!("{device}: 취소했습니다"),
                        _ => format!("{device}: {}{}", v["error"].as_str().unwrap_or("실패했습니다"), if v["request"]["names"].is_array() { " — 그 기기의 파일 창에서 남은 항목을 다시 보낼 수 있습니다" } else { "" }),
                    };
                    notify(&format!("{what} {}", match v["state"].as_str() { Some("completed") => "완료", Some("cancelled") => "취소됨", _ => "실패" }), &text);
                }
                #[cfg(target_os = "macos")]
                let mut v=v;
                #[cfg(target_os = "macos")]
                if v["type"] == "files_native_copy_ready" { plat::native_file_copy_ready(&mut v); }
                #[cfg(target_os = "macos")]
                if v["type"] == "files_native_promise_done" { plat::native_file_promise_done(&v); continue; }
                #[cfg(target_os = "macos")]
                if v["type"] == "files_native_drag_end" { plat::native_file_drag_done(&v); }
                send(v);
            },
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

/// When the home window was last brought forward: devices are checked often only around
/// then. Each check of a Mac without SSH opens a connection to its Screen Sharing, which
/// logs it; every 20 s from every Itda that lists it came to hundreds an hour per Mac.
static HOME_LOOKED: Mutex<Option<std::time::Instant>> = Mutex::new(None);
const PROBE_IN_USE: Duration = Duration::from_secs(30);
const PROBE_IDLE: Duration = Duration::from_secs(10 * 60);
/// How long the home counts as in use after it came forward.
const HOME_IN_USE: Duration = Duration::from_secs(3 * 60);

/// The home window is in front now.
static HOME_FOCUSED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// The home window came forward (`true`; then devices are checked now, unless just done, and
/// often while it stays in front and for a while after) or went behind.
pub fn home_focus(front: bool) {
    HOME_FOCUSED.store(front, std::sync::atomic::Ordering::Relaxed);
    let mut looked = HOME_LOOKED.lock().unwrap();
    let recent = looked.is_some_and(|t| t.elapsed() < Duration::from_secs(15));
    *looked = Some(std::time::Instant::now());
    drop(looked);
    if front && !recent {
        probe_now();
        *SCAN_WAKE.lock().unwrap() = true;
        SCAN_CV.notify_all();
    }
}

/// How often devices are checked (and the tailnet looked over, which also connects to each
/// Mac's Screen Sharing): often while the home is in use, rarely otherwise.
fn probe_interval() -> Duration {
    let in_use = HOME_FOCUSED.load(std::sync::atomic::Ordering::Relaxed) || HOME_LOOKED.lock().unwrap().is_some_and(|t| t.elapsed() < HOME_IN_USE);
    if in_use { PROBE_IN_USE } else { PROBE_IDLE }
}

/// A connection saved from the web viewer (a browser signed in with the PIN) must not be able
/// to send this PC's saved logins somewhere else: pointing an entry at another address (or way
/// in) drops its saved passwords and shared account, and an account can't be attached from
/// there at all. Returns the id whose saved logins to forget.
fn web_save_guard(h: &mut Value, hosts: &[store::Host]) -> Option<String> {
    let id = h["id"].as_str().unwrap_or("").to_string();
    let Some(old) = hosts.iter().find(|x| !id.is_empty() && x.id == id) else {
        h["account"] = json!("");
        return None;
    };
    let new: store::Host = serde_json::from_value(h.clone()).ok()?;
    let to = |x: &store::Host| (x.host.trim().to_lowercase(), x.kind.clone(), x.ssh.clone(), x.hp.clone(), x.lan.trim().to_string());
    if to(&new) == to(old) {
        h["account"] = json!(old.account);
        return None;
    }
    h["account"] = json!("");
    Some(id)
}

/// Forget the passwords saved for a connection (screen, high-performance, files).
fn forget_saved_logins(id: &str) {
    store::set_password(id, None);
    store::set_password(&crate::hp_upgrade::password_key(id), None);
    store::set_password(&format!("files-{id}"), None);
    if let Some(h) = home(|m| m.hosts.iter().find(|h| h.id == id).cloned()) {
        store::set_password(&crate::files::password_key(&h), None);
    }
}

/// Remember the LAN address of saved hosts that Tailscale reaches directly on this network, for
/// when their Tailscale address stops answering. True if any changed.
fn learn_lan(hosts: &mut [store::Host], peers: &[crate::tailscale::Peer]) -> bool {
    let mut changed = false;
    for h in hosts.iter_mut() {
        let (name, _) = crate::tunnel::split_host(h.host.trim(), 0);
        let name = name.to_lowercase();
        let Some(p) = peers.iter().find(|p| [&p.dns, &p.name, &p.ip].iter().any(|n| !n.is_empty() && n.to_lowercase() == name)) else { continue };
        // Only into an empty field: an address someone typed stays.
        if let Some(ip) = crate::tailscale::lan_ip(&p.cur).filter(|_| h.lan.trim().is_empty()) {
            {
                h.lan = ip;
                changed = true;
            }
        }
    }
    changed
}

/// Every 20 s (or on demand): is each Mac's Screen Sharing port answering?
fn start_probes() {
    std::thread::spawn(|| loop {
        // hosts.json is small and only written by the UI thread (atomically): just read it.
        // SSH hosts are reachable when Remote Login answers; the Screen Sharing port may be firewalled.
        let hosts = store::load_hosts();
        let results: HashMap<String, bool> = std::thread::scope(|sc| {
            let hs: Vec<_> = hosts
                .iter()
                .map(|h| {
                    sc.spawn(move || {
                        if store::answers(&h.probe_addr(&h.host)) {
                            return (h.id.clone(), true);
                        }
                        // (Counted only where a connection would really go there.)
                        let lan = h.lan_fallback() && { let a = h.at_lan(); store::answers(&a.probe_addr(&a.host)) };
                        (h.id.clone(), lan)
                    })
                })
                .collect();
            hs.into_iter().filter_map(|h| h.join().ok()).collect()
        });
        post(Event::Online(results));
        let g = PROBE_WAKE.lock().unwrap();
        let (mut g, _) = PROBE_CV.wait_timeout_while(g, probe_interval(), |w| !*w).unwrap();
        *g = false;
    });
}

static ANNOUNCED: Mutex<Option<crate::mdns::Registration>> = Mutex::new(None);

/// Tell other Itdas on this network that Itda runs here, and whether this machine can be
/// opened right now (sharing, Remote Desktop). Called again when that changes.
fn announce() {
    let st = home(|m| m.settings.clone());
    let old = ANNOUNCED.lock().unwrap().take();
    drop(old);
    if !st.announce {
        return;
    }
    #[cfg(windows)]
    let share = crate::share::running().is_some();
    #[cfg(not(windows))]
    let share = false;
    let web = if st.web.on { st.web.port.to_string() } else { String::new() };
    let rdp = if plat::rdp_enabled() { "1" } else { "0" };
    let txt = [("itda", "1"), ("os", plat::PLATFORM), ("v", VERSION), ("share", if share { "1" } else { "0" }), ("rdp", rdp), ("web", web.as_str())];
    let reg = crate::mdns::register("_itda._tcp", st.web.port, &txt);
    crate::log!("announce: itda share={share} rdp={rdp} web={web} ({})", if reg.is_some() { "ok" } else { "failed" });
    *ANNOUNCED.lock().unwrap() = reg;
}

static SCAN_WAKE: Mutex<bool> = Mutex::new(false);
static SCAN_CV: std::sync::Condvar = std::sync::Condvar::new();

fn scan() {
    *SCAN_WAKE.lock().unwrap() = true;
    SCAN_CV.notify_all();
}

/// Bonjour and the tailnet every 30 s (or on demand), so devices that start Itda or sharing,
/// or come online in Tailscale, show up on their own.
fn start_discovery() {
    let me = crate::mdns::this_name().to_ascii_lowercase();
    std::thread::spawn(move || {
        let mut last: Option<Vec<crate::mdns::Found>> = None;
        loop {
            scan_tailnet();
            let mut found = crate::mdns::browse(Duration::from_millis(1500));
            found.retain(|f| f.name.to_ascii_lowercase() != me && !f.hostname.to_ascii_lowercase().starts_with(&format!("{me}.")));
            found.sort_by(|a, b| a.name.cmp(&b.name));
            if last.as_ref() != Some(&found) {
                crate::log!("bonjour: {} found {:?}", found.len(), found.iter().map(|f| &f.host).collect::<Vec<_>>());
                last = Some(found.clone());
            }
            // Always answer: a scan the page asked for waits for this.
            post(Event::Found(found));
            let g = SCAN_WAKE.lock().unwrap();
            let (mut g, _) = SCAN_CV.wait_timeout_while(g, probe_interval(), |w| !*w).unwrap();
            *g = false;
        }
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
        static LAST: Mutex<String> = Mutex::new(String::new());
        let line = format!("tailscale: running={} peers={} https={} serving={serving}", st.running, st.peers.len(), st.https);
        if *LAST.lock().unwrap() != line {
            crate::log!("{line}");
            *LAST.lock().unwrap() = line;
        }
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
    static OUTBOX: Mutex<Vec<(String, Vec<String>)>> = Mutex::new(Vec::new());
    static READY: Condvar = Condvar::new();

    const SHIM: &str = "(()=>{const client=crypto.randomUUID();window.ipc={postMessage:m=>fetch('/ipc?client='+client,{method:'POST',body:m})};\
        (async()=>{for(;;){try{const r=await fetch('/poll?client='+client);for(const m of await r.json())window.itda&&itda.recv(m)}\
        catch(e){await new Promise(r=>setTimeout(r,300))}}})();})();";

    pub fn active() -> bool {
        ACTIVE.load(Relaxed)
    }

    pub fn push(msg: String) {
        for (_, outbox) in OUTBOX.lock().unwrap().iter_mut() { outbox.push(msg.clone()); }
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
        let client = line.split_whitespace().nth(1).and_then(|p| p.split("?client=").nth(1)).unwrap_or("legacy").to_string();
        if path == "/poll" || path == "/ipc" {
            let mut outboxes = OUTBOX.lock().unwrap();
            if !outboxes.iter().any(|(id,_)| id == &client) { outboxes.push((client.clone(), Vec::new())); }
        }
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
                let (mut g, _) = READY.wait_timeout_while(g, Duration::from_secs(10), |o| o.iter().find(|(id,_)| id == &client).is_none_or(|(_,q)| q.is_empty())).unwrap();
                let msgs: Vec<String> = std::mem::take(&mut g.iter_mut().find(|(id,_)| id == &client).unwrap().1);
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

#[cfg(test)]
mod lan_tests {
    #[test]
    fn learns_the_lan_address_of_a_saved_tailscale_host() {
        let peer = |name: &str, dns: &str, cur: &str| crate::tailscale::Peer { name: name.into(), dns: dns.into(), cur: cur.into(), ..Default::default() };
        let mut hosts = vec![
            crate::store::Host { host: "JIUN-HOME.tail.ts.net:3390".into(), ..Default::default() },
            crate::store::Host { host: "other".into(), lan: "10.0.0.9".into(), ..Default::default() },
        ];
        let peers = [peer("jiun-home", "jiun-home.tail.ts.net", "192.168.32.32:41641"), peer("other", "other.tail.ts.net", "")];
        assert!(super::learn_lan(&mut hosts, &peers));
        assert_eq!(hosts[0].lan, "192.168.32.32");
        // A relayed peer (no direct path) keeps what was learned before, and a typed address
        // is never replaced.
        assert_eq!(hosts[1].lan, "10.0.0.9");
        let peers2 = [peer("jiun-home", "jiun-home.tail.ts.net", "192.168.1.50:41641")];
        assert!(!super::learn_lan(&mut hosts, &peers2));
        assert_eq!(hosts[0].lan, "192.168.32.32");
        assert!(!super::learn_lan(&mut hosts, &peers), "nothing new");
    }
}

#[cfg(test)]
mod web_edit_tests {
    use serde_json::json;

    #[test]
    fn web_edits_cannot_redirect_saved_logins() {
        let old = crate::store::Host { id: "a".into(), host: "mac.local".into(), account: "acct".into(), ..Default::default() };
        let hosts = [old.clone()];
        let as_json = |h: &crate::store::Host| serde_json::to_value(h).unwrap();
        // Same address, other details: kept, and the account can't be swapped for another.
        let mut v = as_json(&crate::store::Host { name: "renamed".into(), account: "other".into(), ..old.clone() });
        assert_eq!(super::web_save_guard(&mut v, &hosts), None);
        assert_eq!(v["account"], json!("acct"));
        // Pointed elsewhere (address, or SSH): saved logins go, and so does the account.
        for changed in [
            crate::store::Host { host: "evil.example".into(), ..old.clone() },
            crate::store::Host { ssh: crate::store::Ssh { on: true, user: String::new(), port: 2222 }, ..old.clone() },
        ] {
            let mut v = as_json(&changed);
            assert_eq!(super::web_save_guard(&mut v, &hosts).as_deref(), Some("a"));
            assert_eq!(v["account"], json!(""));
        }
        // A new entry can't take an account's password along.
        let mut v = as_json(&crate::store::Host { id: String::new(), host: "evil.example".into(), account: "acct".into(), ..Default::default() });
        assert_eq!(super::web_save_guard(&mut v, &hosts), None);
        assert_eq!(v["account"], json!(""));
    }
}

#[cfg(test)]
mod files_control_tests {
    use serde_json::json;
    use std::sync::Arc;

    /// The router hands transfer control to files::dispatch before any device or login
    /// lookup: the device-less window can stop its own transfer, another page can't, and an
    /// adopted transfer of a deleted device can still be stopped.
    #[test]
    fn transfer_control_reaches_dispatch_without_a_device() {
        let emit: crate::files::Emit = Arc::new(|_| {});
        let own = crate::files::test_job("__network", "page-own");
        assert!(super::files_control(&json!({"cmd":"files","op":"cancel","id":"__network","window":"page-other","job":own.job}), "__network", emit.clone()));
        assert!(!own.stopped(), "another page stopped this page's transfer");
        assert!(super::files_control(&json!({"cmd":"files","op":"cancel","id":"__network","window":"page-own","job":own.job}), "__network", emit.clone()));
        assert!(own.stopped(), "the device-less window could not stop its own transfer");
        let gone = crate::store::new_id();
        let adopted = crate::files::test_job(&gone, "page-closed");
        crate::files::test_adopt("page-closed", &gone, &adopted.job);
        assert!(super::files_control(&json!({"cmd":"files","op":"cancel_background","id":gone,"window":"page-any","job":adopted.job}), &gone, emit.clone()));
        assert!(adopted.stopped(), "an adopted transfer of a deleted device could not be stopped");
        assert!(!super::files_control(&json!({"cmd":"files","op":"list","id":"x"}), "x", emit));
    }
}

#[cfg(test)]
mod quit_tests {
    use super::*;

    fn drained(reason: QuitReason, left: usize) -> bool {
        QUEUE.lock().unwrap().iter().any(|e| matches!(e, Event::QuitDrained(r, n, _) if *r == reason && *n == left))
    }
    fn ticket() -> u64 { QUIT_PENDING.lock().unwrap().map_or(0, |(t, _)| t) }
    /// Other tests may hold a transfer for a moment: positive outcomes are waited for.
    fn eventually(f: impl Fn() -> bool) -> bool {
        let until = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while std::time::Instant::now() < until { if f() { return true; } std::thread::sleep(std::time::Duration::from_millis(50)); }
        false
    }
    fn clear() {
        QUEUE.lock().unwrap().retain(|e| !matches!(e, Event::QuitDrained(..)));
        *QUIT_PENDING.lock().unwrap() = None;
        QUITTING.store(false, std::sync::atomic::Ordering::SeqCst);
        UPDATING.store(false, std::sync::atomic::Ordering::SeqCst);
        crate::files::open_admissions();
    }

    fn assert_transfer_refused() {
        let events = std::sync::Arc::new(Mutex::new(Vec::<Value>::new()));
        let captured = events.clone();
        crate::files::dispatch(json!({"op":"transfer","from":"local","to":"local","source":"/never-read","destination":"/never-write","names":["fixture"],"policy":"keep","window":"gate-closed"}), Host::default(), store::Login::default(), std::sync::Arc::new(move |v| captured.lock().unwrap().push(v)));
        assert!(events.lock().unwrap().iter().any(|v| v["state"]=="failed" && v["error"]==crate::files::CLOSING), "late transfer was not explicitly refused");
    }

    #[test]
    fn quitting_asks_only_when_something_runs_and_never_the_installer() {
        assert_eq!(quit_step(QuitReason::User, 0), QuitStep::Now);
        assert_eq!(quit_step(QuitReason::Update, 0), QuitStep::Now);
        assert_eq!(quit_step(QuitReason::User, 2), QuitStep::Ask);
        assert_eq!(quit_step(QuitReason::Update, 1), QuitStep::Ask);
        assert_eq!(quit_step(QuitReason::Configuration, 3), QuitStep::StopThenQuit);
        assert_eq!(quit_step(QuitReason::Configuration, 0), QuitStep::Now);
    }

    /// One test, in order: the pending quit, admissions and the job registry are the process's.
    #[test]
    fn waiting_stopping_failing_and_cancelling_a_quit() {
        clear();
        // Wait: nothing happens while a transfer runs, a new one extends it, it goes once all
        // ended; transfers may still start while waiting.
        let a = crate::files::test_job("quit-test", "page");
        quit_choice("wait", QuitReason::User, 0);
        assert!(matches!(*QUIT_PENDING.lock().unwrap(), Some((_, QuitPending::Waiting(QuitReason::User)))));
        assert!(crate::files::accepting());
        std::thread::sleep(std::time::Duration::from_millis(700));
        let b = crate::files::test_job("quit-test", "page");
        a.finish();
        std::thread::sleep(std::time::Duration::from_millis(700));
        assert!(!drained(QuitReason::User, 0) && !a.stopped() && !b.stopped(), "waiting quit or stopped a transfer");
        b.finish();
        assert!(eventually(|| drained(QuitReason::User, 0)), "the quit did not go once transfers ended");
        clear();
        // Cancel: the reservation is gone and transfers may start; a late result is ignored.
        let c = crate::files::test_job("quit-test", "page");
        quit_choice("wait", QuitReason::Update, 0);
        let stale = ticket();
        quit_choice("cancel", QuitReason::Update, 0);
        assert!(QUIT_PENDING.lock().unwrap().is_none() && crate::files::accepting());
        quit_drained(QuitReason::Update, 0, stale);
        assert!(!UPDATING.load(std::sync::atomic::Ordering::SeqCst), "a cancelled update started");
        // A wait replaced by another intent: the old one's report is ignored.
        quit_choice("wait", QuitReason::User, 0);
        let old = ticket();
        quit_choice("wait", QuitReason::Update, 0);
        quit_drained(QuitReason::User, 0, old);
        assert!(matches!(*QUIT_PENDING.lock().unwrap(), Some((_, QuitPending::Waiting(QuitReason::Update)))), "a replaced wait acted");
        quit_choice("cancel", QuitReason::Update, 0);
        // Stop: every transfer is told to stop, nothing new starts, and the quit waits for
        // them to clean up.
        quit_choice("stop", QuitReason::User, 0);
        assert!(c.stopped() && !crate::files::accepting());
        std::thread::sleep(std::time::Duration::from_millis(300));
        assert!(!drained(QuitReason::User, 0), "went before the stopped transfer cleaned up");
        c.finish();
        assert!(eventually(|| drained(QuitReason::User, 0)));
        clear();
        // A stop that doesn't finish in time: Itda stays open (admissions back) and asks again.
        let d = crate::files::test_job("quit-test", "page");
        stop_then(QuitReason::User, std::time::Duration::from_millis(200));
        let t = ticket();
        assert!(eventually(|| QUEUE.lock().unwrap().iter().any(|e| matches!(e, Event::QuitDrained(QuitReason::User, n, _) if *n > 0))));
        quit_drained(QuitReason::User, 1, t);
        assert!(QUIT_PENDING.lock().unwrap().is_none() && crate::files::accepting(), "a stuck stop quit anyway or kept transfers out");
        // The installer's close likewise stays open when cleanup did not finish.
        stop_then(QuitReason::Configuration, std::time::Duration::from_millis(100));
        let t = ticket();
        // A stale stop (cancelled, then a new request) can't act.
        quit_choice("cancel", QuitReason::User, 0);
        quit_choice("wait", QuitReason::User, 0);
        quit_drained(QuitReason::Configuration, 0, t);
        assert!(matches!(*QUIT_PENDING.lock().unwrap(), Some((_, QuitPending::Waiting(QuitReason::User)))), "a stale stop acted");
        quit_choice("cancel", QuitReason::User, 0);
        stop_then(QuitReason::Configuration, std::time::Duration::from_millis(100));
        quit_drained(QuitReason::Configuration, 1, ticket());
        assert!(crate::files::accepting() && QUIT_PENDING.lock().unwrap().is_none());
        drop(d);
        clear();
        // A transfer that starts between the count and the quit is caught: asked instead.
        let e = crate::files::test_job("quit-test", "page");
        assert!(!go_now(QuitReason::User), "quit with a transfer running");
        assert!(crate::files::accepting());
        // An update with a transfer running asks first: nothing is downloaded or replaced.
        assert!(!request_quit(QuitReason::Update));
        assert!(!UPDATING.load(std::sync::atomic::Ordering::SeqCst), "the update started while a transfer ran");
        // Forcing is only for a plain quit; for an update it is a cancel.
        quit_choice("force", QuitReason::Update, 0);
        assert!(crate::files::accepting() && !e.stopped());
        drop(e);
        clear();
        // A stop that reported one left, which then ended before the report was handled:
        // decided on what runs now, so it quits (admissions stay closed) instead of warning.
        let g = crate::files::test_job("quit-test", "page");
        stop_then(QuitReason::User, std::time::Duration::from_millis(100));
        let t = ticket();
        g.finish();
        assert!(eventually(|| crate::files::active_jobs() == 0));
        quit_drained(QuitReason::User, 1, t);
        assert!(QUIT_PENDING.lock().unwrap().is_none() && !crate::files::accepting(), "warned about a transfer that had ended");
        clear();
        // Forcing needs the stuck question's own ticket.
        let h = crate::files::test_job("quit-test", "page");
        ask_quit(QuitReason::User, 1, true);
        let shown = FORCE_TICKET.lock().unwrap().expect("a stuck quit offers force");
        quit_choice("force", QuitReason::User, shown + 1000);
        assert!(crate::files::accepting(), "forced with a wrong ticket");
        ask_quit(QuitReason::User, 1, true);
        let shown = FORCE_TICKET.lock().unwrap().unwrap();
        quit_choice("force", QuitReason::User, shown);
        assert!(!crate::files::accepting(), "the matching force did not quit");
        drop(h);
        clear();
        // A bounded wait gives up on a transfer that does not end.
        let f = crate::files::test_job("quit-test", "page");
        assert!(drain_jobs(std::time::Duration::from_millis(200), None) >= 1);
        drop(f);
        clear();
        // Permission relaunch does not stop/wait/force or launch anything while
        // a transfer accepted after a UI's earlier idle observation is present.
        let launch = std::cell::Cell::new(0);
        let quit = std::cell::Cell::new(0);
        assert_eq!(crate::files::active_jobs(), 0);
        let busy = crate::files::test_job("permission-restart", "page");
        let result = permission_restart_with(|| { launch.set(launch.get()+1); Ok(()) }, || quit.set(quit.get()+1));
        assert!(result.unwrap_err().contains("전송"));
        assert_eq!((launch.get(), quit.get()), (0, 0));
        assert!(crate::files::accepting() && !busy.stopped());
        busy.finish();
        // Launch failure restores only the admission reservation owned here.
        let result = permission_restart_with(|| { assert!(!crate::files::accepting()); Err("fake launch failure".into()) }, || quit.set(quit.get()+1));
        assert_eq!(result.unwrap_err(), "fake launch failure");
        assert!(crate::files::accepting() && quit.get()==0);
        crate::files::close_admissions();
        assert!(permission_restart_with(|| panic!("launched through a closed gate"), || panic!("quit through a closed gate")).is_err());
        assert!(!crate::files::accepting(), "permission helper reopened someone else's gate");
        crate::files::open_admissions();
        let intent = pend(QuitPending::Waiting(QuitReason::Update));
        assert!(permission_restart_with(|| panic!("replaced pending quit"), || panic!("quit over pending intent")).is_err());
        assert_eq!(*QUIT_PENDING.lock().unwrap(), Some((intent, QuitPending::Waiting(QuitReason::Update))));
        assert!(crate::files::accepting());
        clear();
        UPDATING.store(true, std::sync::atomic::Ordering::SeqCst);
        assert!(permission_restart_with(|| panic!("launched during an update"), || panic!("quit over an update")).is_err());
        assert!(UPDATING.load(std::sync::atomic::Ordering::SeqCst) && crate::files::accepting());
        UPDATING.store(false, std::sync::atomic::Ordering::SeqCst);
        // Once reserved idle, even a late transfer attempt inside the fake launch
        // cannot enter, and only a successful launch commits the injected quit.
        permission_restart_with(|| {
            assert!(!crate::files::accepting());
            let events = std::sync::Arc::new(Mutex::new(Vec::<Value>::new()));
            let captured = events.clone();
            crate::files::dispatch(json!({"op":"transfer","from":"local","to":"local","source":"/never-read","destination":"/never-write","names":["fixture"],"policy":"keep","window":"permission-late"}), Host::default(), store::Login::default(), std::sync::Arc::new(move |v| captured.lock().unwrap().push(v)));
            assert_eq!(crate::files::active_jobs(), 0, "late transfer bypassed admission close");
            assert!(events.lock().unwrap().iter().any(|v| v["state"]=="failed" && v["error"]==crate::files::CLOSING), "late request was not explicitly refused");
            launch.set(launch.get()+1);
            Ok(())
        }, || { assert!(!crate::files::accepting()); quit.set(quit.get()+1); }).unwrap();
        assert_eq!((launch.get(), quit.get()), (1, 1));
        assert!(!crate::files::accepting() && QUITTING.load(std::sync::atomic::Ordering::SeqCst));
        // Even a late updater failure cannot reopen the committed permission quit.
        update_install_failed();
        assert!(!crate::files::accepting());
        assert_transfer_refused();
        clear();
        // Consume the wait intent into a fake updater. Both ownership and removal
        // of the stale Cancel bar occur before any install/quit callback.
        pend(QuitPending::Waiting(QuitReason::Update));
        let before = QUEUE.lock().unwrap().len();
        let installed = std::cell::Cell::new(false);
        assert!(go_now_with(QuitReason::Update, || installed.set(true), || panic!("quit instead of updating")));
        assert!(installed.get() && UPDATING.load(std::sync::atomic::Ordering::SeqCst));
        assert!(QUIT_PENDING.lock().unwrap().is_none() && !crate::files::accepting());
        assert!(QUEUE.lock().unwrap().iter().skip(before).any(|e| matches!(e, Event::Page(v) if v["type"]=="quit_pending" && v["count"]==0)), "consumed update left its Cancel bar visible");
        let before_cancel = QUEUE.lock().unwrap().len();
        quit_choice("cancel", QuitReason::Update, 0);
        assert!(!crate::files::accepting(), "stale Cancel reopened updater-owned admission");
        assert!(QUEUE.lock().unwrap().iter().skip(before_cancel).any(|e| matches!(e, Event::Page(v) if v["type"]=="quit_pending" && v["count"]==0 && v["cancelled"]==false)), "stale Cancel falsely claimed an active intent was cancelled");
        assert_transfer_refused();
        assert!(!request_quit_with(QuitReason::User, |_| panic!("quit during in-process update")));
        assert!(!request_quit_with(QuitReason::Configuration, |_| panic!("installer quit during update")));
        assert!(!request_quit_with(QuitReason::Update, |_| panic!("second install during update")));
        let in_flight = crate::files::test_job("fake-update-owned", "page");
        quit_choice("stop", QuitReason::User, 0);
        quit_choice("wait", QuitReason::User, 0);
        assert!(!in_flight.stopped() && QUIT_PENDING.lock().unwrap().is_none(), "a stale choice interrupted the updater");
        in_flight.finish();
        assert!(!go_now_with(QuitReason::User, || {}, || panic!("go_now quit during update")));
        update_install_failed();
        assert!(!UPDATING.load(std::sync::atomic::Ordering::SeqCst) && crate::files::accepting());
        clear();
        // Failure cleanup must not steal a stop intent's gate.
        let stopping = pend(QuitPending::Stopping(QuitReason::User));
        crate::files::close_admissions();
        UPDATING.store(true, std::sync::atomic::Ordering::SeqCst);
        update_install_failed();
        assert_eq!(*QUIT_PENDING.lock().unwrap(), Some((stopping, QuitPending::Stopping(QuitReason::User))));
        assert!(!crate::files::accepting());
        assert_transfer_refused();
        clear();
        // Conversely, a bounded stop cleanup cannot steal an updater's gate.
        let stuck = crate::files::test_job("updater-stop-overlap", "page");
        stop_then(QuitReason::User, std::time::Duration::from_millis(100));
        let stop_ticket = ticket();
        UPDATING.store(true, std::sync::atomic::Ordering::SeqCst);
        quit_drained(QuitReason::User, 1, stop_ticket);
        assert!(!crate::files::accepting());
        assert_transfer_refused();
        stuck.finish();
        update_install_failed();
        assert!(crate::files::accepting());
        clear();
        // Any final committed quit keeps its reservation across stale Cancel or
        // a late update-error callback. No actual quit/update callback is used.
        assert!(go_now_with(QuitReason::User, || {}, || quit.set(quit.get()+1)));
        assert!(QUITTING.load(std::sync::atomic::Ordering::SeqCst));
        UPDATING.store(true, std::sync::atomic::Ordering::SeqCst);
        update_install_failed();
        quit_choice("cancel", QuitReason::User, 0);
        assert!(!crate::files::accepting());
        assert_transfer_refused();
        clear();
        // A release removed during Wait never reserves a closed updater gate or
        // launches a network/update/quit effect. The check itself is injected.
        let checked = std::cell::Cell::new(0);
        pend(QuitPending::Waiting(QuitReason::Update));
        assert!(!go_now_target::<()>(QuitReason::Update, None, || checked.set(checked.get()+1), |_| panic!("installed a missing release"), || panic!("quit for a missing release")));
        assert!(crate::files::accepting() && QUIT_PENDING.lock().unwrap().is_none());
        assert!(!UPDATING.load(std::sync::atomic::Ordering::SeqCst) && checked.get()==1);
        crate::files::close_admissions();
        assert!(!go_now_target::<()>(QuitReason::Update, None, || checked.set(checked.get()+1), |_| panic!("installed no release"), || panic!("quit with someone else's gate")));
        assert!(!crate::files::accepting(), "missing release reopened a pre-existing closed gate");
        let other_stop = pend(QuitPending::Stopping(QuitReason::User));
        assert!(!go_now_target::<()>(QuitReason::Update, None, || checked.set(checked.get()+1), |_| panic!("installed no release"), || panic!("quit over another stop")));
        assert_eq!(*QUIT_PENDING.lock().unwrap(), Some((other_stop, QuitPending::Stopping(QuitReason::User))));
        assert!(!crate::files::accepting() && checked.get()==3);
        clear();
    }
}
