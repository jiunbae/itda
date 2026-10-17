//! Itda for Mac: one native window (tao + WKWebView via wry) showing the web viewer, served
//! by the in-process gateway in local mode (127.0.0.1, signed in with a one-time token).
//! Everything else (RFB/ARD login, SSH tunnel, RDP bridge, Keychain) is the portable core.

use muda::{AboutMetadata, Menu, PredefinedMenuItem, Submenu};
use tao::dpi::LogicalSize;
use tao::event::{Event, WindowEvent};
use tao::event_loop::{ControlFlow, EventLoopBuilder};
use tao::window::WindowBuilder;
use wry::WebViewBuilder;

/// The app menu. Without Edit items, ⌘C/⌘V/⌘A don't work in WKWebView text fields.
fn menu() -> Menu {
    let about = AboutMetadata {
        name: Some(crate::APP.into()),
        version: Some(env!("CARGO_PKG_VERSION").into()),
        comments: Some("내 기기를 잇다".into()),
        website: Some("https://jiun.dev/itda/".into()),
        ..Default::default()
    };
    let app = Submenu::with_items(
        crate::APP,
        true,
        &[
            &PredefinedMenuItem::about(None, Some(about)),
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
    let window = Submenu::with_items(
        "윈도우",
        true,
        &[&PredefinedMenuItem::minimize(None), &PredefinedMenuItem::maximize(None), &PredefinedMenuItem::separator(), &PredefinedMenuItem::close_window(None)],
    );
    let m = Menu::new();
    for s in [app, edit, view, window].iter().flatten() {
        let _ = m.append(s);
    }
    m
}

pub fn run() {
    let url = match crate::gateway::start_local() {
        Ok(u) => u,
        Err(e) => {
            crate::log!("gateway: {e}");
            eprintln!("Itda: {e}");
            return;
        }
    };
    // Test hook: serve without a window and print the sign-in URL (automated checks only).
    if std::env::var_os("ITDA_TEST_LOCAL").is_some() {
        println!("{url}");
        loop {
            std::thread::sleep(std::time::Duration::from_secs(3600));
        }
    }
    let event_loop = EventLoopBuilder::new().build();
    let menu = menu();
    menu.init_for_nsapp();
    let window = WindowBuilder::new()
        .with_title(crate::APP)
        .with_inner_size(LogicalSize::new(1100.0, 760.0))
        .with_min_inner_size(LogicalSize::new(420.0, 520.0))
        .build(&event_loop)
        .expect("window");
    let webview = WebViewBuilder::new()
        .with_url(&url)
        .with_devtools(std::env::var_os("ITDA_DEVTOOLS").is_some())
        .with_back_forward_navigation_gestures(false)
        // Links that leave the app (help pages, Tailscale) open in the default browser.
        .with_navigation_handler(|u| {
            if u.starts_with("http://127.0.0.1:") || u.starts_with("about:") {
                return true;
            }
            if u.starts_with("https://") {
                let _ = std::process::Command::new("/usr/bin/open").arg(&u).spawn();
            }
            false
        })
        .with_new_window_req_handler(|u, _| {
            if u.starts_with("https://") {
                let _ = std::process::Command::new("/usr/bin/open").arg(&u).spawn();
            }
            wry::NewWindowResponse::Deny
        })
        .build(&window)
        .expect("webview");
    crate::log!("window up");
    event_loop.run(move |event, _, flow| {
        *flow = ControlFlow::Wait;
        let _ = (&webview, &menu);
        if let Event::WindowEvent { event: WindowEvent::CloseRequested, .. } = event {
            crate::gateway::stop();
            *flow = ControlFlow::Exit;
        }
    });
}
