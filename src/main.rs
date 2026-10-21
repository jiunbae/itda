//! Itda (잇다) — fast, lightweight remote desktop for macOS Screen Sharing and other VNC servers.
//! Windows: WebView2 home + native viewer windows. macOS: WKWebView home + web viewer windows.
#![windows_subsystem = "windows"]
// The portable core carries the Windows viewer's protocol paths too; on macOS only the
// gateway's subset is used.
#![cfg_attr(not(windows), allow(dead_code))]

// Portable core: protocols, storage, discovery, the web gateway.
#[cfg(test)]
mod cu_e2e;
#[cfg(target_os = "macos")]
mod h264_mac;
#[cfg(target_os = "macos")]
mod host_mac;
mod hp_upgrade;
mod update;
mod remote_setup;
/// Itda's high-performance sharing port on a Mac.
pub const fn host_mac_port() -> u16 {
    5910
}
#[cfg(all(test, target_os = "macos"))]
mod h264_e2e;
mod autostart;
mod demand;
mod enc;
mod files;
mod gateway;
mod outbox;
mod img;
mod log;
mod mdns;
mod rdp;
mod rfb;
mod store;
mod tailscale;
mod tunnel;
mod ws;

// Windows app: home window, native session windows, "Share this PC".
#[cfg(windows)]
mod clip;
#[cfg(windows)]
mod dxgi;
#[cfg(windows)]
mod fb;
#[cfg(windows)]
mod h264_win;
#[cfg(windows)]
mod files_win;
#[cfg(windows)]
mod keys;
#[cfg(windows)]
mod net;
#[cfg(windows)]
mod session;
#[cfg(any(windows, target_os = "macos"))]
mod server;
#[cfg(windows)]
mod share;
#[cfg(windows)]
mod toolbar;
#[cfg(windows)]
mod ui;

// The home window (address book, settings, help) on both platforms.
#[cfg(any(windows, target_os = "macos"))]
mod home;

/// Product name shown to people.
pub const APP: &str = "Itda";

const USAGE: &str = "itda [host[:port]] [-u user] [-p password] [--swap]

인자 없이 실행하면 Itda 홈(주소록)이 열립니다.
  host         주소만 주면 홈에서 바로 그 Mac에 연결합니다
  -u user      Mac 계정 이름 (Apple 계정 로그인)
  -p password  비밀번호 (또는 ITDA_PASSWORD 환경변수) — 주면 홈 없이 바로 접속
  --swap       Ctrl ↔ Cmd 바꾸기

접속 중 단축키 (Ctrl+Alt+…): Enter 전체 화면 · 0 맞춤 · 1 실제 크기 · S 통계 · V 보기 전용
                           P 스크린샷 · I 연결 정보 · T 툴바 고정 · R 재연결 · M 메뉴";

/// ssh asking for the password: answer and leave before touching logs or windows.
fn askpass_mode() {
    if std::env::args().nth(1).as_deref() == Some("--askpass") || std::env::var_os("SSH_ASKPASS_REQUIRE").is_some() && std::env::var_os("ITDA_ASKPASS_SECRET").is_some() {
        let prompt = std::env::args().last().unwrap_or_default();
        std::process::exit(tunnel::askpass(&prompt));
    }
}

#[cfg(not(windows))]
fn main() {
    askpass_mode();
    if remote_setup::import_mode() { return; }
    // Read-only check in a fresh instance of the same executable. Do not initialize
    // sharing, the home window, or a permission request in this mode.
    #[cfg(target_os = "macos")]
    if std::env::args().nth(1).as_deref() == Some("--permission-status") {
        let (screen, access) = host_mac::permissions();
        println!("{}", serde_json::json!({"screen":screen,"access":access}));
        return;
    }
    // Itda's high-performance sharing on this Mac (started by another Itda over SSH, or by hand).
    #[cfg(target_os = "macos")]
    if std::env::args().nth(1).as_deref() == Some("--hp-server") {
        log::init("hp-server");
        let port = std::env::args().nth(2).and_then(|p| p.parse().ok()).unwrap_or(host_mac::PORT);
        host_mac::serve_forever(port);
    }
    #[cfg(target_os = "macos")]
    if std::env::args().nth(1).as_deref() == Some("--export-file-locations") {
        let root = files::locations::folder();
        match files::locations::write(&store::load_hosts(), &root) {
            Ok(()) => println!("{}", root.display()),
            Err(e) => { eprintln!("{e}"); std::process::exit(1); }
        }
        return;
    }
    log::init(if cfg!(debug_assertions) && std::env::var_os("ITDA_NATIVE_VIEWER_TEST").is_some() { "monitor-test" } else { "" });
    std::panic::set_hook(Box::new(|p| crate::log!("PANIC: {p}\n{}", std::backtrace::Backtrace::force_capture())));
    #[cfg(target_os="macos")]
    let _configuration_lock=match remote_setup::application_lock() {
        Ok(lock)=>lock,Err(e)=>{crate::log!("configuration: {e}");return;}
    };
    #[cfg(target_os = "macos")]
    if host_mac::enabled() {
        host_mac::start_background();
    }
    #[cfg(target_os = "macos")]
    home::run(None);
    #[cfg(not(target_os = "macos"))]
    eprintln!("Itda runs on Windows and macOS.");
}

#[cfg(windows)]
fn main() {
    askpass_mode();
    if remote_setup::import_mode() { return; }
    let mut args = std::env::args().skip(1).peekable();
    let managed = args.peek().map(String::as_str) == Some("--session");
    log::init(if managed { "session" } else { "" });
    std::panic::set_hook(Box::new(|p| {
        crate::log!("PANIC: {p}");
        session::emit(serde_json::json!({"ev": "error", "kind": "other", "msg": format!("내부 오류: {p}")}));
        if !session::managed() {
            ui::message(&format!("내부 오류로 종료합니다:\n{p}\n\n로그: {}", log::path().display()), true);
        }
    }));
    if managed {
        return session::run_managed();
    }

    let mut host = None;
    let mut user = None;
    let mut pass = std::env::var("ITDA_PASSWORD").or_else(|_| std::env::var("MVNC_PASSWORD")).ok();
    let mut swap = false;
    while let Some(a) = args.next() {
        match a.as_str() {
            "-u" => user = args.next(),
            "-p" => pass = args.next(),
            "--swap" => swap = true,
            "--login" => autostart::AT_LOGIN.store(true, std::sync::atomic::Ordering::Relaxed),
            _ if host.is_none() && !a.starts_with('-') => host = Some(a),
            _ => return ui::message(USAGE, false),
        }
    }

    // Host + password on the command line: connect straight away, no home window.
    if let (Some(h), Some(p)) = (&host, &pass) {
        let st = store::load_settings();
        return session::run(session::Launch {
            title: h.clone(),
            host: h.clone(),
            user: user.unwrap_or_default(),
            pass: p.clone(),
            opts: store::HostOpts { swap: swap || st.defaults.swap, ..st.defaults },
            hotkey: st.hotkey,
            toolbar: st.toolbar,
            mac_scroll: st.mac_scroll,
            scroll_accel: st.scroll_accel,
            ..Default::default()
        });
    }
    home::run(host);
}
