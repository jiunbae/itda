//! Win32 viewer window: blits the framebuffer (1:1 BitBlt or HALFTONE fit-scaling), forwards
//! input, and draws the session chrome (toolbar, reconnect card, stats pill, toasts).

use crate::fb::dib_info;
use crate::keys;
use crate::net::{self, sh, Cursor, WM_CLIP, WM_CURSOR, WM_FB, WM_NETERR, WM_RESIZED, WM_STATUS};
use crate::rfb::{Creds, Session, RX_BYTES};
use crate::session::{self, Launch};
use crate::toolbar::{self, Item};
use serde_json::json;
use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::ptr::{null, null_mut};
use std::sync::atomic::{AtomicBool, Ordering::Relaxed};
use std::time::{Duration, Instant};
use windows_sys::Win32::Foundation::*;
use windows_sys::Win32::Graphics::Dwm::DwmSetWindowAttribute;
use windows_sys::Win32::Graphics::Gdi::*;
use windows_sys::Win32::System::DataExchange::*;
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::System::Memory::{GlobalAlloc, GlobalLock, GlobalUnlock, GMEM_MOVEABLE};
use windows_sys::Win32::UI::Controls::SetScrollInfo;
use windows_sys::Win32::UI::HiDpi::GetDpiForWindow;
use windows_sys::Win32::UI::Input::Ime::ImmAssociateContext;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::*;
use windows_sys::Win32::UI::WindowsAndMessaging::*;

/// A live session plus what's needed to re-establish it.
pub struct Conn {
    pub sess: Session,
    /// Shown to people ("alex@mac.local (SSH)").
    pub addr: String,
    pub dial: net::Dial,
    /// Traffic is encrypted (SSH tunnel).
    pub secure: bool,
    pub creds: Creds,
}

static SWAP: AtomicBool = AtomicBool::new(false);
static VIEW_ONLY: AtomicBool = AtomicBool::new(false);

// Command ids (system-menu safe: low 4 bits zero, below 0xF000).
const M_FIT: usize = 0x110;
const M_ACTUAL: usize = 0x120;
const M_FULL: usize = 0x130;
const M_STATS: usize = 0x140;
const M_VIEWONLY: usize = 0x150;
const M_HANGUL: usize = 0x160;
const M_SWAP: usize = 0x170;
const M_RECONNECT: usize = 0x180;
const M_CLIP: usize = 0x190;
const M_REMOTE_RES: usize = 0x1a0;
const M_SHOT: usize = 0x1b0;
const M_INFO: usize = 0x1c0;
const M_DISCONNECT: usize = 0x1d0;
const M_PIN: usize = 0x1e0;
const M_SCALE: usize = 0x1f0;
const M_SCREEN_ALL: usize = 0x200;
const M_SCREEN0: usize = 0x210; // + 0x10 per display, up to 16
const MAX_SCREENS: usize = 16;
const M_LAYOUT0: usize = 0x300; // + 0x10 per monitor_layout value 0..=4
const M_WINDOWS: usize = 0x360;
const M_GATHER: usize = 0x370;
const M_KEY0: usize = 0x400; // + 0x10 per special key
// Toolbar-only dropdowns.
const TB_SCREENS: usize = 0x900;
const TB_KEYS: usize = 0x910;
const TB_MORE: usize = 0x920;

const CF_UNICODETEXT: u32 = 13;
const TIMER_TICK: usize = 1;
const DWMWA_USE_IMMERSIVE_DARK_MODE: u32 = 20;

const CMD: u32 = keys::XK_SUPER_L;
const CTRL: u32 = keys::XK_CONTROL_L;
const OPT: u32 = 0xffe9;
const SHIFT: u32 = 0xffe1;

/// Mac shortcuts that Windows would otherwise swallow or that are awkward to type.
const SPECIAL_KEYS: &[(&str, &[u32])] = &[
    ("⌘ Space — Spotlight", &[CMD, 0x20]),
    ("⌘ Tab — 앱 전환", &[CMD, 0xff09]),
    ("⌘ Q — 앱 종료", &[CMD, 0x71]),
    ("⌘ W — 창 닫기", &[CMD, 0x77]),
    ("⌘ H — 앱 가리기", &[CMD, 0x68]),
    ("⌥⌘ Esc — 강제 종료", &[OPT, CMD, 0xff1b]),
    ("⌃⌘ Q — 화면 잠금", &[CTRL, CMD, 0x71]),
    ("⇧⌘ 3 — Mac 전체 화면 캡처", &[SHIFT, CMD, 0x33]),
    ("⇧⌘ 4 — Mac 영역 캡처", &[SHIFT, CMD, 0x34]),
    ("⌃ ↑ — Mission Control", &[CTRL, 0xff52]),
    ("⌃ ↓ — 앱 Exposé", &[CTRL, 0xff54]),
    ("F11 — 데스크탑 보기", &[0xffc8]),
    ("⌃ ← — 왼쪽 데스크탑", &[CTRL, 0xff51]),
    ("⌃ → — 오른쪽 데스크탑", &[CTRL, 0xff53]),
];

struct Fonts {
    text: HFONT,
    bold: HFONT,
    title: HFONT,
    icon: HFONT,
}

#[derive(Default)]
struct Ui {
    /// Desktop name reported by the server.
    name: String,
    addr: String,
    secure: bool,
    l: Launch,
    buttons: u8,
    pos: (u16, u16),
    wheel: (i32, i32),
    /// Last notch sent to a Mac (time, direction) and how many came in quick succession.
    wheel_last: Option<(Instant, i32)>,
    wheel_run: u32,
    /// key id -> keysym sent on press, so the release matches.
    keys: HashMap<u32, u32>,
    cursor: Option<HCURSOR>, // None = system arrow, Some(null) = hidden
    cursor_img: Option<Cursor>,
    saved: Option<(isize, WINDOWPLACEMENT)>,
    fonts: Option<Fonts>,
    stats_line: String,
    last: (u64, u64), // (bytes, updates) at the previous tick
    /// Last clipboard text exchanged either way (suppresses echo).
    last_clip: String,
    /// Items in the stock system menu (ours are appended after them).
    sys_count: i32,
    toast: Option<(String, Instant)>,
    started: Option<Instant>,
    first_frame: Option<Instant>,
    thumb_done: bool,
    /// "닫기" button on the reconnect card, in client coordinates.
    close_btn: [i32; 4],
}

/// View geometry lives outside the RefCell: it's read from nested window messages.
#[derive(Clone, Copy, Default)]
struct View {
    actual: bool,
    sx: i32,
    sy: i32,
    /// Show only this part of the framebuffer (one display): [x, y, w, h].
    crop: Option<[i32; 4]>,
}

/// What the window paints from: the UI thread's own copy of the framebuffer. Changed parts
/// are copied over from the decoder's under a short lock, and scaling (HALFTONE StretchBlt,
/// milliseconds for a big window) runs without it, so it never holds the decoder up.
struct Front {
    hbm: HBITMAP,
    bits: *mut u32,
    w: usize,
    h: usize,
}

impl Front {
    /// Make it w×h; true when it had to be (re)made (and holds nothing yet).
    fn fit(&mut self, w: usize, h: usize) -> bool {
        if (w, h) == (self.w, self.h) && !self.hbm.is_null() {
            return false;
        }
        unsafe {
            if !self.hbm.is_null() {
                DeleteObject(self.hbm);
            }
            let bi = dib_info(w.max(1) as i32, h.max(1) as i32);
            let mut bits = null_mut();
            self.hbm = CreateDIBSection(null_mut(), &bi, DIB_RGB_COLORS, &mut bits, null_mut(), 0);
            self.bits = if self.hbm.is_null() { null_mut() } else { bits as *mut u32 };
        }
        (self.w, self.h) = if self.bits.is_null() { (0, 0) } else { (w, h) };
        true
    }
}

impl Drop for Front {
    fn drop(&mut self) {
        if !self.hbm.is_null() {
            unsafe { DeleteObject(self.hbm) };
        }
    }
}

/// A window showing one display (or screen region) of the same session: no second
/// connection, it paints from the same copy of the framebuffer and sends input to the same
/// session. Owned by the session window; see open_windows.
struct Child {
    hwnd: HWND,
    /// The region it shows, by Region::id: it stays on that display when displays are
    /// reordered, and closes when that display goes.
    region: u32,
    view: View,
    /// Style and placement before full screen.
    saved: Option<(isize, WINDOWPLACEMENT)>,
}

thread_local! {
    static CHILDREN: RefCell<Vec<Child>> = const { RefCell::new(Vec::new()) };
    /// The view of a display window while CreateWindowExW is still sending it messages.
    static CREATING: Cell<View> = Cell::new(View::default());
    /// The session is closing: windows going away must not try to close it again.
    static ENDING: Cell<bool> = const { Cell::new(false) };
    /// The remembered monitor_mode was applied (once the displays were known).
    static APPLIED: Cell<bool> = const { Cell::new(false) };
    /// Region::id the session window is cropped to (None: everything).
    static MAIN_REGION: Cell<Option<u32>> = const { Cell::new(None) };
    static FRONT: RefCell<Front> = const { RefCell::new(Front { hbm: null_mut(), bits: null_mut(), w: 0, h: 0 }) };
    static UI: RefCell<Ui> = RefCell::new(Ui::default());
    static VIEW: Cell<View> = Cell::new(View::default());
    static IN_SCROLL: Cell<bool> = const { Cell::new(false) };
}

fn ui<R>(f: impl FnOnce(&mut Ui) -> R) -> R {
    UI.with(|u| f(&mut u.borrow_mut()))
}
fn view() -> View {
    VIEW.with(|v| v.get())
}

/// The view of window `h`: the session window's, or a display window's own (fit, cropped).
fn view_of(h: HWND) -> View {
    if h as isize == sh().hwnd.load(Relaxed) {
        return view();
    }
    CHILDREN.with(|c| c.try_borrow().ok().and_then(|c| c.iter().find(|w| w.hwnd == h).map(|w| w.view)))
        .unwrap_or_else(|| CREATING.with(|c| c.get()))
}
fn set_view(v: View) {
    VIEW.with(|c| c.set(v))
}

pub fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(Some(0)).collect()
}

pub fn message(text: &str, error: bool) {
    let flags = if error { MB_ICONERROR } else { MB_ICONINFORMATION };
    unsafe { MessageBoxW(null_mut(), wide(text).as_ptr(), wide(crate::APP).as_ptr(), flags) };
}

fn hk() -> &'static str {
    if ui(|u| u.l.hotkey == "ctrl_shift") { "Ctrl+Shift" } else { "Ctrl+Alt" }
}

unsafe fn make_fonts(dpi: i32) -> Fonts {
    let f = |px: i32, weight: i32, face: &str| CreateFontW(-(px * dpi / 96), 0, 0, 0, weight, 0, 0, 0, 1, 0, 0, 5, 0, wide(face).as_ptr());
    let icons = if std::path::Path::new(r"C:\Windows\Fonts\SegoeIcons.ttf").exists() { "Segoe Fluent Icons" } else { "Segoe MDL2 Assets" };
    Fonts { text: f(13, 400, "Segoe UI"), bold: f(13, 600, "Segoe UI"), title: f(17, 600, "Segoe UI"), icon: f(28, 400, icons) }
}

unsafe fn free_fonts(f: Fonts) {
    for h in [f.text, f.bold, f.title, f.icon] {
        DeleteObject(h);
    }
}

pub fn run(c: Conn, l: Launch) {
    let Conn { sess, addr, dial, secure, creds } = c;
    SWAP.store(l.opts.swap, Relaxed);
    VIEW_ONLY.store(l.opts.view_only, Relaxed);
    net::init(sess.writer, sess.width, sess.height);
    sh().auto_reconnect.store(l.opts.reconnect, Relaxed);
    set_view(View { actual: l.opts.actual, ..View::default() });
    let toolbar_mode = if l.toolbar.is_empty() { "auto".to_string() } else { l.toolbar.clone() };
    let start_full = l.opts.fullscreen;
    ui(|u| {
        u.name = sess.name.clone();
        u.addr = addr.clone();
        u.secure = secure;
        if l.title.is_empty() {
            u.l = Launch { title: sess.name.clone(), ..l };
        } else {
            u.l = l;
        }
        u.started = Some(Instant::now());
    });
    unsafe {
        let hinst = GetModuleHandleW(null());
        let cls = wide("itda_session");
        let icon = LoadIconW(hinst, 1 as _);
        let wc = WNDCLASSEXW {
            cbSize: size_of::<WNDCLASSEXW>() as u32,
            lpfnWndProc: Some(wndproc),
            hInstance: hinst,
            hIcon: icon,
            hIconSm: icon,
            hCursor: LoadCursorW(null_mut(), IDC_ARROW),
            lpszClassName: cls.as_ptr(),
            ..std::mem::zeroed()
        };
        RegisterClassExW(&wc);
        let child_cls = wide(CHILD_CLASS);
        RegisterClassExW(&WNDCLASSEXW { lpfnWndProc: Some(child_proc), lpszClassName: child_cls.as_ptr(), ..wc });

        // Client area = remote size, shrunk (aspect kept) to fit the work area.
        let style = WS_OVERLAPPEDWINDOW | WS_CLIPCHILDREN;
        let (fw, fh) = (sess.width.max(1) as i32, sess.height.max(1) as i32);
        let mut rc = RECT { left: 0, top: 0, right: fw, bottom: fh };
        AdjustWindowRectEx(&mut rc, style, 0, 0);
        let (bw, bh) = (rc.right - rc.left - fw, rc.bottom - rc.top - fh);
        let mut wa: RECT = std::mem::zeroed();
        SystemParametersInfoW(SPI_GETWORKAREA, 0, &mut wa as *mut _ as _, 0);
        let s = (((wa.right - wa.left - bw) as f64 / fw as f64).min((wa.bottom - wa.top - bh) as f64 / fh as f64)).min(1.0);
        let (ww, wh) = ((fw as f64 * s) as i32 + bw, (fh as f64 * s) as i32 + bh);
        let (wx, wy) = (wa.left + (wa.right - wa.left - ww) / 2, wa.top + (wa.bottom - wa.top - wh) / 2);

        let h = CreateWindowExW(0, cls.as_ptr(), wide(crate::APP).as_ptr(), style, wx, wy, ww, wh, null_mut(), null_mut(), hinst, null());
        if h.is_null() {
            return message("창을 만들 수 없습니다", true);
        }
        let dark: BOOL = 1;
        DwmSetWindowAttribute(h, DWMWA_USE_IMMERSIVE_DARK_MODE, &dark as *const BOOL as _, 4);
        let fonts = make_fonts(GetDpiForWindow(h) as i32);
        let sys_count = GetMenuItemCount(GetSystemMenu(h, 0));
        ui(|u| {
            u.fonts = Some(fonts);
            u.sys_count = sys_count;
        });
        ImmAssociateContext(h, null_mut()); // raw keys, no IME composition
        sh().hwnd.store(h as isize, Relaxed);
        SetWindowsHookExW(WH_KEYBOARD_LL, Some(kb_hook), hinst, 0);
        toolbar::create(h, toolbar_items(), &toolbar_mode);
        AddClipboardFormatListener(h);
        update_title(h);
        update_scroll(h);
        SetTimer(h, TIMER_TICK, 1000, None);
        ShowWindow(h, SW_SHOW);
        SetForegroundWindow(h);
        if start_full {
            toggle_fullscreen(h);
        }
        apply_monitor_mode(); // a manual split is known now; server displays come later
        session::emit(json!({"ev": "connected", "name": sess.name, "w": sess.width, "h": sess.height, "hwnd": h as isize}));

        net::spawn(sess.reader, dial, creds);

        let mut msg: MSG = std::mem::zeroed();
        while GetMessageW(&mut msg, null_mut(), 0, 0) > 0 {
            DispatchMessageW(&msg); // no TranslateMessage: we never want WM_CHAR
        }
    }
    session::emit(json!({"ev": "closed"}));
}

fn toolbar_items() -> Vec<Item> {
    let p = hk();
    // Tooltip strings live for the whole session.
    let t = |s: String| -> &'static str { Box::leak(s.into_boxed_str()) };
    vec![
        Item::Button(M_FULL, t(format!("전체 화면 ({p}+Enter)"))),
        Item::Button(M_SCALE, t(format!("창에 맞춤 ↔ 실제 크기 ({p}+0 / {p}+1)"))),
        Item::Button(TB_SCREENS, "모니터 선택"),
        Item::Sep,
        Item::Button(TB_KEYS, "Mac 특수 키 보내기"),
        Item::Button(M_CLIP, "클립보드 공유"),
        Item::Button(M_VIEWONLY, t(format!("보기 전용 ({p}+V)"))),
        Item::Sep,
        Item::Button(M_SHOT, t(format!("스크린샷 저장 ({p}+P)"))),
        Item::Button(M_STATS, t(format!("통계 표시 ({p}+S)"))),
        Item::Button(TB_MORE, t(format!("더보기 ({p}+M)"))),
        Item::Button(M_PIN, t(format!("툴바 고정 ({p}+T)"))),
        Item::Sep,
        Item::Button(M_DISCONNECT, "연결 끊기"),
    ]
}

/// How a toolbar button looks right now.
pub struct Look {
    pub glyph: char,
    pub on: bool,
    pub danger: bool,
    pub enabled: bool,
}

pub fn look(id: usize) -> Look {
    let (opts, full) = ui(|u| (u.l.opts, u.saved.is_some()));
    let v = view();
    let lk = |glyph, on| Look { glyph, on, danger: false, enabled: true };
    match id {
        M_FULL => lk(if full { '\u{E73F}' } else { '\u{E740}' }, false),
        M_SCALE => lk(if v.actual { '\u{E71E}' } else { '\u{E9A6}' }, v.actual),
        // Always on: it also holds the manual split, for servers that don't report displays.
        TB_SCREENS => Look { enabled: true, ..lk('\u{E7F4}', v.crop.is_some() || !child_hwnds().is_empty()) },
        TB_KEYS => lk('\u{E765}', false),
        M_CLIP => lk('\u{E77F}', opts.clipboard),
        M_VIEWONLY => lk('\u{E7B3}', VIEW_ONLY.load(Relaxed)),
        M_SHOT => lk('\u{E722}', false),
        M_STATS => lk('\u{E9D9}', opts.stats),
        TB_MORE => lk('\u{E712}', false),
        M_PIN => {
            let pinned = toolbar::mode() == "pinned";
            lk(if pinned { '\u{E77A}' } else { '\u{E718}' }, pinned)
        }
        M_DISCONNECT => Look { danger: true, ..lk('\u{E711}', false) },
        _ => lk(' ', false),
    }
}

/// A toolbar button was clicked; `pt` is where a dropdown should open (screen coords).
pub fn toolbar_click(h: HWND, id: usize, pt: POINT) {
    unsafe {
        let menu = match id {
            TB_SCREENS => screens_menu(),
            TB_KEYS => keys_menu(),
            TB_MORE => full_menu(h),
            _ => {
                command(h, id);
                return;
            }
        };
        let cmd = TrackPopupMenu(menu, TPM_RETURNCMD | TPM_LEFTALIGN | TPM_TOPALIGN, pt.x, pt.y, 0, h, null()) as usize;
        DestroyMenu(menu);
        if cmd != 0 {
            command(h, cmd);
        }
    }
}

// ---------------------------------------------------------------- layout

/// Where the framebuffer lands in the client area: scale and offset.
struct Layout {
    s: f64,
    /// Source offset of the shown region inside the framebuffer.
    cx: i32,
    cy: i32,
    ox: i32,
    oy: i32,
    dw: i32,
    dh: i32,
    fw: i32,
    fh: i32,
}

unsafe fn client(h: HWND) -> (i32, i32) {
    let mut rc: RECT = std::mem::zeroed();
    GetClientRect(h, &mut rc);
    (rc.right.max(1), rc.bottom.max(1))
}

unsafe fn layout(h: HWND, full_w: i32, full_h: i32) -> Layout {
    let (cw, ch) = client(h);
    let v = view_of(h);
    let (cx, cy, fw, fh) = match v.crop {
        Some([x, y, w, hh]) if x + w <= full_w && y + hh <= full_h => (x, y, w, hh),
        _ => (0, 0, full_w, full_h),
    };
    if v.actual {
        let off = |c: i32, f: i32, scroll: i32| if f <= c { (c - f) / 2 } else { -scroll.clamp(0, f - c) };
        return Layout { s: 1.0, cx, cy, ox: off(cw, fw, v.sx), oy: off(ch, fh, v.sy), dw: fw, dh: fh, fw, fh };
    }
    let (fw1, fh1) = (fw.max(1), fh.max(1));
    let s = (cw as f64 / fw1 as f64).min(ch as f64 / fh1 as f64);
    let dw = (fw1 as f64 * s).round() as i32;
    let dh = (fh1 as f64 * s).round() as i32;
    Layout { s, cx, cy, ox: (cw - dw) / 2, oy: (ch - dh) / 2, dw, dh, fw, fh }
}

unsafe fn cur_layout(h: HWND) -> Layout {
    let (w, h2) = sh().size();
    layout(h, w as i32, h2 as i32)
}

/// Scrollbars exist only in 1:1 mode when the remote screen is larger than the window.
unsafe fn update_scroll(h: HWND) {
    if IN_SCROLL.with(|c| c.replace(true)) {
        return; // SetScrollInfo can resize the client area and re-enter via WM_SIZE
    }
    let v = view();
    let l = cur_layout(h);
    let (fw, fh) = (l.fw, l.fh);
    for _ in 0..2 {
        // twice: one bar appearing can make the other necessary
        let (cw, ch) = client(h);
        for (bar, total, page, pos) in [(SB_HORZ, fw, cw, v.sx), (SB_VERT, fh, ch, v.sy)] {
            let si = SCROLLINFO {
                cbSize: size_of::<SCROLLINFO>() as u32,
                fMask: SIF_RANGE | SIF_PAGE | SIF_POS,
                nMin: 0,
                nMax: if v.actual { total - 1 } else { 0 },
                nPage: if v.actual { page as u32 } else { 0 },
                nPos: pos,
                nTrackPos: 0,
            };
            SetScrollInfo(h, bar, &si, 1);
        }
    }
    let pos = |bar| {
        let mut si = SCROLLINFO { cbSize: size_of::<SCROLLINFO>() as u32, fMask: SIF_POS, ..std::mem::zeroed() };
        GetScrollInfo(h, bar, &mut si);
        si.nPos
    };
    set_view(View { sx: pos(SB_HORZ), sy: pos(SB_VERT), ..v });
    IN_SCROLL.with(|c| c.set(false));
    InvalidateRect(h, null(), 0);
}

unsafe fn on_scroll(h: HWND, bar: SCROLLBAR_CONSTANTS, code: i32) {
    let mut si = SCROLLINFO { cbSize: size_of::<SCROLLINFO>() as u32, fMask: SIF_ALL, ..std::mem::zeroed() };
    GetScrollInfo(h, bar, &mut si);
    let line = 48;
    si.nPos = match code {
        0 => si.nPos - line,            // SB_LINEUP
        1 => si.nPos + line,            // SB_LINEDOWN
        2 => si.nPos - si.nPage as i32, // SB_PAGEUP
        3 => si.nPos + si.nPage as i32, // SB_PAGEDOWN
        4 | 5 => si.nTrackPos,          // SB_THUMBPOSITION / SB_THUMBTRACK
        6 => 0,                         // SB_TOP
        7 => si.nMax,                   // SB_BOTTOM
        _ => return,
    };
    si.fMask = SIF_POS;
    SetScrollInfo(h, bar, &si, 1);
    GetScrollInfo(h, bar, &mut si);
    let mut v = view();
    if bar == SB_HORZ {
        v.sx = si.nPos;
    } else {
        v.sy = si.nPos;
    }
    set_view(v);
    InvalidateRect(h, null(), 0);
}

// ---------------------------------------------------------------- window proc

unsafe extern "system" fn wndproc(h: HWND, m: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    match m {
        WM_PAINT => paint(h),
        WM_PRINTCLIENT => {
            let mut rc: RECT = std::mem::zeroed();
            GetClientRect(h, &mut rc);
            draw(h, wp as HDC, rc);
        }
        WM_ERASEBKGND => return 1,
        WM_SIZE => {
            update_scroll(h);
            rebuild_cursor(h);
            toolbar::layout();
        }
        WM_RESIZED => {
            update_scroll(h);
            rebuild_cursor(h);
            update_title(h);
            toolbar::refresh();
            refit_children();
            apply_monitor_mode();
        }
        WM_CLIP => {
            if let Some(t) = sh().clip_in.lock().unwrap().take() {
                if ui(|u| u.l.opts.clipboard) {
                    set_clipboard(h, &t);
                }
            }
        }
        WM_CLIPBOARDUPDATE => clip_out(h),
        WM_SETFOCUS => {
            clip_out(h);
            return DefWindowProcW(h, m, wp, lp);
        }
        WM_HSCROLL => on_scroll(h, SB_HORZ, (wp & 0xffff) as i32),
        WM_VSCROLL => on_scroll(h, SB_VERT, (wp & 0xffff) as i32),
        WM_FB => on_fb(h),
        WM_CURSOR => {
            let c = sh().cursor.lock().unwrap().take();
            if c.is_some() {
                ui(|u| u.cursor_img = c);
                rebuild_cursor(h);
            }
        }
        WM_STATUS => {
            update_title(h);
            InvalidateRect(h, null(), 0); // reconnect card appears / disappears
            for c in child_hwnds() {
                update_title(c);
                InvalidateRect(c, null(), 0);
            }
        }
        WM_TIMER if wp == TIMER_TICK => tick(h),
        WM_NETERR => {
            let e = sh().error.lock().unwrap().clone().unwrap_or_default();
            if session::managed() {
                let kind = if e.contains("인증") { "auth" } else { "lost" };
                session::emit(json!({"ev": "error", "kind": kind, "msg": e}));
            } else {
                let t = wide(&format!("{e}\n\n로그: {}", crate::log::path().display()));
                MessageBoxW(h, t.as_ptr(), wide(crate::APP).as_ptr(), MB_ICONERROR);
            }
            end_session(h);
        }
        WM_INITMENUPOPUP if (lp >> 16) & 0xffff != 0 => rebuild_system_menu(h),
        WM_SYSCOMMAND if command(h, wp & 0xfff0) => {}
        WM_SETCURSOR if (lp & 0xffff) as u32 == HTCLIENT => {
            SetCursor(ui(|u| u.cursor).unwrap_or_else(|| LoadCursorW(null_mut(), IDC_ARROW)));
            return 1;
        }
        WM_MOUSEMOVE | WM_LBUTTONDOWN | WM_LBUTTONUP | WM_MBUTTONDOWN | WM_MBUTTONUP | WM_RBUTTONDOWN
        | WM_RBUTTONUP => mouse(h, m, lp),
        WM_MOUSEWHEEL | WM_MOUSEHWHEEL => wheel(m == WM_MOUSEHWHEEL, ((wp >> 16) & 0xffff) as i16 as i32),
        WM_KEYDOWN | WM_SYSKEYDOWN | WM_KEYUP | WM_SYSKEYUP => {
            if !key(h, m, wp, lp) {
                return DefWindowProcW(h, m, wp, lp);
            }
        }
        WM_KILLFOCUS => release_all(),
        WM_CAPTURECHANGED => release_buttons(),
        WM_DPICHANGED => {
            let dpi = ((wp >> 16) & 0xffff) as i32;
            let old = ui(|u| u.fonts.replace(make_fonts(dpi)));
            if let Some(f) = old {
                free_fonts(f);
            }
            toolbar::dpi_changed(dpi);
            let r = &*(lp as *const RECT);
            SetWindowPos(h, null_mut(), r.left, r.top, r.right - r.left, r.bottom - r.top, SWP_NOZORDER | SWP_NOACTIVATE);
        }
        WM_CLOSE => close(h),
        WM_DESTROY => PostQuitMessage(0),
        WM_TEST_SNAP => test_snap(h, wp),
        _ => return DefWindowProcW(h, m, wp, lp),
    }
    0
}

/// Test hook (only with ITDA_TEST_SNAP=<dir>): render the window, toolbar included, to
/// <dir>\snap-<n>.png. Cross-process capture can't see GDI output on a headless desktop.
const WM_TEST_SNAP: u32 = WM_APP + 30;

unsafe fn test_snap(h: HWND, n: usize) {
    let Some(dir) = std::env::var_os("ITDA_TEST_SNAP") else { return };
    let (cw, ch) = client(h);
    let hdc = GetDC(h);
    let grab = |w: i32, hh: i32, f: &dyn Fn(HDC)| -> Vec<u32> {
        let mdc = CreateCompatibleDC(hdc);
        let bi = dib_info(w, hh);
        let mut bits = null_mut();
        let bmp = CreateDIBSection(null_mut(), &bi, DIB_RGB_COLORS, &mut bits, null_mut(), 0);
        let old = SelectObject(mdc, bmp);
        f(mdc);
        GdiFlush();
        let px = std::slice::from_raw_parts(bits as *const u32, (w * hh) as usize).to_vec();
        SelectObject(mdc, old);
        DeleteObject(bmp);
        DeleteDC(mdc);
        px
    };
    let mut px = grab(cw, ch, &|dc| draw(h, dc, RECT { left: 0, top: 0, right: cw, bottom: ch }));
    let tb = FindWindowExW(h, null_mut(), wide("itda_tb").as_ptr(), null());
    if !tb.is_null() && GetWindowLongW(tb, GWL_STYLE) as u32 & WS_VISIBLE != 0 {
        let mut r: RECT = std::mem::zeroed();
        GetWindowRect(tb, &mut r);
        let mut pt = POINT { x: r.left, y: r.top };
        ScreenToClient(h, &mut pt);
        let (tw, th) = (r.right - r.left, r.bottom - r.top);
        let t = grab(tw, th, &|dc| {
            SendMessageW(tb, WM_PRINTCLIENT, dc as usize, PRF_CLIENT as isize);
        });
        for y in 0..th {
            for x in 0..tw {
                let (dx, dy) = (pt.x + x, pt.y + y);
                if dx >= 0 && dy >= 0 && dx < cw && dy < ch {
                    px[(dy * cw + dx) as usize] = t[(y * tw + x) as usize];
                }
            }
        }
    }
    ReleaseDC(h, hdc);
    let img = crate::img::Image { w: cw as usize, h: ch as usize, px: px.iter().map(|p| p & 0xff_ffff).collect() };
    let _ = std::fs::write(std::path::Path::new(&dir).join(format!("snap-{n}.png")), img.png(6));
}

/// A window was closed. A display window just goes; the session window hides while display
/// windows remain (the session stays up for them) and ends the session when it's the last.
unsafe fn close(h: HWND) {
    if h as isize != sh().hwnd.load(Relaxed) {
        DestroyWindow(h);
        return;
    }
    save_thumb();
    if child_hwnds().is_empty() {
        ENDING.with(|e| e.set(true));
        DestroyWindow(h);
    } else {
        release_all();
        ShowWindow(h, SW_HIDE);
    }
}

/// Leave the session from any window ("연결 끊기", lost link): every window closes.
unsafe fn end_session(h: HWND) {
    let _ = h;
    if ENDING.with(|e| e.replace(true)) {
        return;
    }
    save_thumb();
    // Display windows are owned by the session window and go with it.
    DestroyWindow(sh().hwnd.load(Relaxed) as HWND);
}

// ---------------------------------------------------------------- menus & commands

unsafe fn add(m: HMENU, id: usize, text: &str, checked: bool, enabled: bool) {
    let mut f = MF_STRING;
    if checked {
        f |= MF_CHECKED;
    }
    if !enabled {
        f |= MF_GRAYED;
    }
    AppendMenuW(m, f, id, wide(text).as_ptr());
}

unsafe fn sep(m: HMENU) {
    AppendMenuW(m, MF_SEPARATOR, 0, null());
}

unsafe fn sub(m: HMENU, popup: HMENU, text: &str) {
    AppendMenuW(m, MF_POPUP, popup as usize, wide(text).as_ptr());
}

unsafe fn screens_menu() -> HMENU {
    let m = CreatePopupMenu();
    let rs = regions();
    let crop = view().crop;
    let layout = ui(|u| u.l.opts.monitor_layout);
    add(m, M_SCREEN_ALL, "전체 화면 영역", crop.is_none(), true);
    if rs.is_empty() {
        let note = if sh().screens.lock().unwrap().is_empty() { "모니터 정보 없음 — 아래 ‘화면 나누기’로 나눌 수 있습니다" } else { "모니터 1개" };
        add(m, 0, note, false, false);
    }
    for (i, r) in rs.iter().take(MAX_SCREENS).enumerate() {
        add(m, M_SCREEN0 + i * 0x10, &region_name(r, i), crop == Some(r.rect), true);
    }
    sep(m);
    add(m, M_WINDOWS, "각 화면을 창으로 열기", false, rs.len() >= 2);
    add(m, M_GATHER, "한 창으로 모으기", false, !child_hwnds().is_empty());
    let split = CreatePopupMenu();
    for (i, label) in LAYOUTS.iter().enumerate() {
        add(split, M_LAYOUT0 + i * 0x10, label, layout as usize == i, true);
    }
    sub(m, split, "화면 나누기");
    m
}

unsafe fn keys_menu() -> HMENU {
    let m = CreatePopupMenu();
    let ro = VIEW_ONLY.load(Relaxed);
    for (i, (label, _)) in SPECIAL_KEYS.iter().enumerate() {
        add(m, M_KEY0 + i * 0x10, label, false, !ro);
    }
    m
}

/// Everything, for the "더보기" button, Ctrl+Alt+M and the window's system menu.
unsafe fn fill_menu(h: HWND, m: HMENU) {
    let p = hk();
    let (opts, full) = ui(|u| (u.l.opts, u.saved.is_some()));
    let v = view();
    let has_screens = !sh().screens.lock().unwrap().is_empty();
    let pinned = toolbar::mode() == "pinned";
    let _ = h;
    add(m, M_FIT, &format!("창에 맞춤\t{p}+0"), !v.actual, true);
    add(m, M_ACTUAL, &format!("실제 크기 (1:1)\t{p}+1"), v.actual, true);
    add(m, M_FULL, &format!("전체 화면\t{p}+Enter"), full, true);
    sub(m, screens_menu(), "모니터");
    add(m, M_REMOTE_RES, "Mac 해상도를 창 크기로 변경", false, has_screens);
    sep(m);
    add(m, M_VIEWONLY, &format!("보기 전용\t{p}+V"), VIEW_ONLY.load(Relaxed), true);
    add(m, M_SWAP, "Ctrl ↔ ⌘ Cmd 바꾸기", SWAP.load(Relaxed), true);
    add(m, M_HANGUL, "한/영 키 → Caps Lock (해제 시 Ctrl+Space)", opts.hangul_caps, true);
    sub(m, keys_menu(), "Mac 특수 키 보내기");
    sep(m);
    add(m, M_CLIP, "클립보드 공유", opts.clipboard, true);
    add(m, M_STATS, &format!("통계 표시\t{p}+S"), opts.stats, true);
    add(m, M_PIN, &format!("툴바 고정\t{p}+T"), pinned, true);
    sep(m);
    add(m, M_SHOT, &format!("스크린샷 저장\t{p}+P"), false, true);
    add(m, M_INFO, &format!("연결 정보\t{p}+I"), false, true);
    add(m, M_RECONNECT, &format!("다시 연결\t{p}+R"), false, true);
    add(m, M_DISCONNECT, "연결 끊기", false, true);
}

unsafe fn full_menu(h: HWND) -> HMENU {
    let m = CreatePopupMenu();
    fill_menu(h, m);
    m
}

/// The system menu is rebuilt each time it opens, so checks are always current.
unsafe fn rebuild_system_menu(h: HWND) {
    let m = GetSystemMenu(h, 0);
    let base = ui(|u| u.sys_count);
    while GetMenuItemCount(m) > base {
        DeleteMenu(m, base as u32, MF_BYPOSITION); // also destroys our submenus
    }
    sep(m);
    fill_menu(h, m);
}

unsafe fn show_menu(h: HWND) {
    let mut pt = POINT { x: 0, y: 0 };
    GetCursorPos(&mut pt);
    let m = full_menu(h);
    let id = TrackPopupMenu(m, TPM_RETURNCMD | TPM_RIGHTBUTTON, pt.x, pt.y, 0, h, null()) as usize;
    DestroyMenu(m);
    if id != 0 {
        command(h, id);
    }
}

fn emit_opts() {
    let opts = ui(|u| u.l.opts);
    session::emit(json!({"ev": "opts", "opts": opts}));
}

/// Handle a viewer command; false if `id` isn't ours.
unsafe fn command(h: HWND, id: usize) -> bool {
    let on_off = |b: bool| if b { "켜짐" } else { "꺼짐" };
    match id {
        M_FIT | M_ACTUAL | M_SCALE => {
            let actual = if id == M_SCALE { !view().actual } else { id == M_ACTUAL };
            set_view(View { actual, sx: 0, sy: 0, ..view() });
            ui(|u| u.l.opts.actual = actual);
            update_scroll(h);
            rebuild_cursor(h);
            emit_opts();
            toast(h, if actual { "실제 크기 (1:1)" } else { "창에 맞춤" });
        }
        M_FULL => toggle_fullscreen(h),
        M_STATS => {
            ui(|u| {
                u.l.opts.stats = !u.l.opts.stats;
                u.stats_line.clear();
            });
            tick(h);
            InvalidateRect(h, &stats_area(h), 0);
            emit_opts();
        }
        M_VIEWONLY => {
            release_all();
            let v = !VIEW_ONLY.load(Relaxed);
            VIEW_ONLY.store(v, Relaxed);
            ui(|u| u.l.opts.view_only = v);
            update_title(h);
            emit_opts();
            toast(h, &format!("보기 전용 {}", on_off(v)));
        }
        M_HANGUL => {
            let v = ui(|u| {
                u.l.opts.hangul_caps = !u.l.opts.hangul_caps;
                u.l.opts.hangul_caps
            });
            emit_opts();
            toast(h, if v { "한/영 키 → Caps Lock" } else { "한/영 키 → Ctrl+Space" });
        }
        M_SWAP => {
            release_all();
            let v = !SWAP.load(Relaxed);
            SWAP.store(v, Relaxed);
            ui(|u| u.l.opts.swap = v);
            emit_opts();
            toast(h, &format!("Ctrl ↔ ⌘ 바꾸기 {}", on_off(v)));
        }
        M_RECONNECT => sh().reconnect(),
        M_CLIP => {
            let v = ui(|u| {
                u.l.opts.clipboard = !u.l.opts.clipboard;
                u.l.opts.clipboard
            });
            emit_opts();
            toast(h, &format!("클립보드 공유 {}", on_off(v)));
        }
        M_REMOTE_RES => {
            let (cw, ch) = client(h);
            sh().set_desktop_size(cw as u16, ch as u16);
        }
        M_SHOT => screenshot(h),
        M_INFO => info(h),
        M_DISCONNECT => end_session(h),
        M_PIN => {
            let mode = if toolbar::mode() == "pinned" { "auto" } else { "pinned" };
            toolbar::set_mode(mode);
            session::emit(json!({"ev": "toolbar", "mode": mode}));
        }
        M_SCREEN_ALL => select_screen(h, None),
        M_WINDOWS => open_windows(),
        M_GATHER => select_screen(h, None),
        _ if (M_LAYOUT0..M_LAYOUT0 + 0x10 * LAYOUTS.len()).contains(&id) => {
            let layout = ((id - M_LAYOUT0) / 0x10) as u8;
            gather();
            // A new division: the remembered display may not exist in it; start from the first.
            ui(|u| {
                u.l.opts.monitor_layout = layout;
                u.l.opts.monitor_id = None;
            });
            emit_opts();
            APPLIED.with(|a| a.set(false));
            apply_monitor_mode();
            toast(h, LAYOUTS[layout as usize]);
        }
        _ if (M_SCREEN0..M_SCREEN0 + 0x10 * MAX_SCREENS).contains(&id) => select_screen(h, Some((id - M_SCREEN0) / 0x10)),
        _ if (M_KEY0..M_KEY0 + 0x10 * SPECIAL_KEYS.len()).contains(&id) => {
            if !VIEW_ONLY.load(Relaxed) {
                let (label, combo) = SPECIAL_KEYS[(id - M_KEY0) / 0x10];
                combo.iter().for_each(|&k| sh().key(true, k));
                combo.iter().rev().for_each(|&k| sh().key(false, k));
                toast(h, &format!("보냄: {label}"));
            }
        }
        _ => return false,
    }
    toolbar::refresh();
    true
}

unsafe fn update_title(h: HWND) {
    let title = ui(|u| u.l.title.clone());
    let st = sh().status.lock().unwrap().clone();
    let (w, hh) = sh().size();
    let mut t = if st.is_empty() { format!("{title} — {w}×{hh}") } else { format!("{title} — {st}") };
    if VIEW_ONLY.load(Relaxed) {
        t.push_str(" [보기 전용]");
    }
    if h as isize != sh().hwnd.load(Relaxed) {
        let region = CHILDREN.with(|c| c.borrow().iter().find(|w| w.hwnd == h).map(|w| w.region));
        if let Some(id) = region {
            let rs = regions();
            if let Some(i) = rs.iter().position(|r| r.id == id) {
                t = format!("{} — {t}", region_name(&rs[i], i));
            }
        }
    }
    t.push_str(" — Itda");
    SetWindowTextW(h, wide(&t).as_ptr());
}

/// Once a second: stats line, toast expiry, first thumbnail.
unsafe fn tick(h: HWND) {
    let (bytes, upd) = (RX_BYTES.load(Relaxed), sh().updates.load(Relaxed));
    let rtt = sh().rtt_ms();
    let (stats, toast_gone, thumb_due) = ui(|u| {
        let (b0, u0) = std::mem::replace(&mut u.last, (bytes, upd));
        if u.l.opts.stats {
            let mbps = (bytes - b0) as f64 * 8.0 / 1e6;
            let rtt = rtt.map_or("-".into(), |r| format!("{r:.0} ms"));
            u.stats_line = format!("{} fps   {mbps:.1} Mbps   RTT {rtt}", upd - u0);
        }
        let gone = u.toast.as_ref().is_some_and(|(_, t)| t.elapsed() > Duration::from_millis(2500));
        if gone {
            u.toast = None;
        }
        let due = !u.thumb_done && u.first_frame.is_some_and(|t| t.elapsed() > Duration::from_secs(3));
        (u.l.opts.stats, gone, due)
    });
    if stats {
        InvalidateRect(h, &stats_area(h), 0);
    }
    if toast_gone {
        InvalidateRect(h, &toast_area(h), 0);
    }
    if thumb_due {
        save_thumb();
    }
}

/// The user picked region `i` (None: everything) for the session window: show it and
/// remember it (monitor_mode 0 + monitor_id, or 1).
unsafe fn select_screen(_from: HWND, i: Option<usize>) {
    let id = i.and_then(|i| regions().get(i).map(|r| r.id));
    if i.is_some() && id.is_none() {
        return;
    }
    APPLIED.with(|a| a.set(true)); // the user's choice beats the remembered one arriving later
    if !child_hwnds().is_empty() {
        gather();
    }
    ui(|u| {
        u.l.opts.monitor_mode = if id.is_some() { 0 } else { 1 };
        if id.is_some() {
            u.l.opts.monitor_id = id;
        }
    });
    emit_opts();
    crop_main(i);
}

/// Show region `i` (None: everything) in the session window.
unsafe fn crop_main(i: Option<usize>) {
    let r = i.and_then(|i| regions().get(i).copied());
    MAIN_REGION.with(|m| m.set(r.map(|r| r.id)));
    let crop = r.map(|r| r.rect);
    let h = sh().hwnd.load(Relaxed) as HWND;
    let reshape = view().crop != crop;
    set_view(View { crop, sx: 0, sy: 0, ..view() });
    if reshape {
        let (w, h2) = sh().size();
        let [_, _, cw, ch] = crop.unwrap_or([0, 0, w as i32, h2 as i32]);
        fit_shape(h, cw, ch);
    }
    update_scroll(h);
    rebuild_cursor(h);
    InvalidateRect(h, null(), 0);
}

/// A normal (not maximized, not full screen) session window takes the shape of what it now
/// shows — one display instead of all of them side by side — keeping its centre, within its
/// monitor's work area, no larger than 1:1.
unsafe fn fit_shape(h: HWND, fw: i32, fh: i32) {
    if fw < 1 || fh < 1 || IsZoomed(h) != 0 || IsWindowVisible(h) == 0 || ui(|u| u.saved.is_some()) {
        return;
    }
    let mut rc = RECT { left: 0, top: 0, right: fw, bottom: fh };
    AdjustWindowRectEx(&mut rc, GetWindowLongW(h, GWL_STYLE) as u32, 0, GetWindowLongW(h, GWL_EXSTYLE) as u32);
    let (bw, bh) = (rc.right - rc.left - fw, rc.bottom - rc.top - fh);
    let mut mi: MONITORINFO = std::mem::zeroed();
    mi.cbSize = size_of::<MONITORINFO>() as u32;
    GetMonitorInfoW(MonitorFromWindow(h, MONITOR_DEFAULTTONEAREST), &mut mi);
    let wa = mi.rcWork;
    let s = (((wa.right - wa.left - bw) as f64 / fw as f64).min((wa.bottom - wa.top - bh) as f64 / fh as f64)).min(1.0);
    let (ww, wh) = ((fw as f64 * s) as i32 + bw, (fh as f64 * s) as i32 + bh);
    let mut cur: RECT = std::mem::zeroed();
    GetWindowRect(h, &mut cur);
    let x = ((cur.left + cur.right) / 2 - ww / 2).clamp(wa.left, (wa.right - ww).max(wa.left));
    let y = ((cur.top + cur.bottom) / 2 - wh / 2).clamp(wa.top, (wa.bottom - wh).max(wa.top));
    SetWindowPos(h, null_mut(), x, y, ww, wh, SWP_NOZORDER | SWP_NOACTIVATE);
}

// ---------------------------------------------------------------- displays and their windows

const CHILD_CLASS: &str = "itda_display";

/// "화면 나누기" choices, by HostOpts::monitor_layout.
const LAYOUTS: [&str; 5] = ["서버가 알려 준 모니터", "가로로 2개", "세로로 2개", "가로로 3개", "2×2"];

/// A part of the framebuffer a view can show: a display the server reported, or a screen
/// region from a manual split (named as such: it isn't a detected monitor).
#[derive(Clone, Copy, PartialEq, Debug)]
struct Region {
    rect: [i32; 4],
    manual: bool,
    /// HostOpts::monitor_id of this region: the server's screen id, or the index of a manual one.
    id: u32,
}

/// The regions to offer: the manual split if one is chosen, else the server's displays when
/// it reports more than one. Never guessed from the picture's shape.
fn regions() -> Vec<Region> {
    let (w, h) = sh().size();
    let layout = ui(|u| u.l.opts.monitor_layout);
    if let Some(parts) = split(layout, w as i32, h as i32) {
        return parts.into_iter().enumerate().map(|(i, rect)| Region { rect, manual: true, id: i as u32 }).collect();
    }
    let screens = sh().screens.lock().unwrap().clone();
    if screens.len() < 2 {
        return Vec::new();
    }
    screens.iter().map(|s| Region { rect: [s.x as i32, s.y as i32, s.w as i32, s.h as i32], manual: false, id: s.id }).collect()
}

/// A manual split of a w×h framebuffer (monitor_layout 1–4) into equal regions, left to
/// right then top to bottom, covering it exactly; None for 0 (server displays).
fn split(layout: u8, w: i32, h: i32) -> Option<Vec<[i32; 4]>> {
    let (cols, rows) = match layout {
        1 => (2, 1),
        2 => (1, 2),
        3 => (3, 1),
        4 => (2, 2),
        _ => return None,
    };
    if w < cols || h < rows {
        return None;
    }
    Some((0..rows).flat_map(|r| (0..cols).map(move |c| {
        let (x0, x1) = (w * c / cols, w * (c + 1) / cols);
        let (y0, y1) = (h * r / rows, h * (r + 1) / rows);
        [x0, y0, x1 - x0, y1 - y0]
    })).collect())
}

fn region_name(r: &Region, i: usize) -> String {
    if r.manual {
        format!("화면 영역 {} — {}×{}", i + 1, r.rect[2], r.rect[3])
    } else {
        format!("모니터 {} — {}×{}", i + 1, r.rect[2], r.rect[3])
    }
}

fn child_hwnds() -> Vec<HWND> {
    CHILDREN.with(|c| c.borrow().iter().map(|w| w.hwnd).collect())
}

/// Work areas of this PC's monitors, left to right then top to bottom.
unsafe fn local_work_areas() -> Vec<RECT> {
    unsafe extern "system" fn each(m: HMONITOR, _: HDC, _: *mut RECT, data: LPARAM) -> BOOL {
        let list = &mut *(data as *mut Vec<RECT>);
        let mut mi: MONITORINFO = std::mem::zeroed();
        mi.cbSize = size_of::<MONITORINFO>() as u32;
        if GetMonitorInfoW(m, &mut mi) != 0 {
            list.push(mi.rcWork);
        }
        1
    }
    let mut list: Vec<RECT> = Vec::new();
    EnumDisplayMonitors(null_mut(), null(), Some(each), &mut list as *mut _ as LPARAM);
    list.sort_by_key(|r| (r.left, r.top));
    list
}

/// One window per region: the session window keeps the first, a display window opens for
/// each other one on another monitor of this PC — not the session window's own (cascaded
/// over it when there are fewer monitors than displays).
unsafe fn open_windows() {
    let main = sh().hwnd.load(Relaxed) as HWND;
    let rs = regions();
    if rs.len() < 2 {
        return toast(main, "나눌 화면이 없습니다 — ‘화면 나누기’에서 고를 수 있습니다");
    }
    APPLIED.with(|a| a.set(true));
    gather();
    crop_main(Some(0));
    let full = ui(|u| {
        u.l.opts.monitor_mode = 2;
        u.l.opts.monitor_fullscreen
    });
    emit_opts();
    let mut mi: MONITORINFO = std::mem::zeroed();
    mi.cbSize = size_of::<MONITORINFO>() as u32;
    GetMonitorInfoW(MonitorFromWindow(main, MONITOR_DEFAULTTONEAREST), &mut mi);
    let own = mi.rcWork;
    let same = |a: &RECT| (a.left, a.top, a.right, a.bottom) == (own.left, own.top, own.right, own.bottom);
    let areas: Vec<RECT> = local_work_areas().into_iter().filter(|a| !same(a)).collect();
    let style = WS_OVERLAPPEDWINDOW | WS_CLIPCHILDREN;
    let cls = wide(CHILD_CLASS);
    let hinst = GetModuleHandleW(null());
    for (i, r) in rs.iter().enumerate().skip(1).take(MAX_SCREENS - 1) {
        let k = i - 1;
        let wa = areas.get(k).copied().unwrap_or(own);
        let cascade = if k < areas.len() { 0 } else { 40 * (k - areas.len() + 1) as i32 };
        let (fw, fh) = (r.rect[2].max(1), r.rect[3].max(1));
        let mut rc = RECT { left: 0, top: 0, right: fw, bottom: fh };
        AdjustWindowRectEx(&mut rc, style, 0, WS_EX_APPWINDOW);
        let (bw, bh) = (rc.right - rc.left - fw, rc.bottom - rc.top - fh);
        let s = (((wa.right - wa.left - bw - cascade) as f64 / fw as f64).min((wa.bottom - wa.top - bh - cascade) as f64 / fh as f64)).clamp(0.1, 1.0);
        let (ww, wh) = ((fw as f64 * s) as i32 + bw, (fh as f64 * s) as i32 + bh);
        let (wx, wy) = (wa.left + (wa.right - wa.left - ww) / 2 + cascade, wa.top + (wa.bottom - wa.top - wh) / 2 + cascade);
        let view = View { crop: Some(r.rect), ..View::default() };
        CREATING.with(|c| c.set(view));
        // Owned by the session window: it closes with the session, has its own taskbar button.
        let h = CreateWindowExW(WS_EX_APPWINDOW, cls.as_ptr(), wide(crate::APP).as_ptr(), style, wx, wy, ww, wh, main, null_mut(), hinst, null());
        if h.is_null() {
            continue;
        }
        let dark: BOOL = 1;
        DwmSetWindowAttribute(h, DWMWA_USE_IMMERSIVE_DARK_MODE, &dark as *const BOOL as _, 4);
        ImmAssociateContext(h, null_mut());
        CHILDREN.with(|c| c.borrow_mut().push(Child { hwnd: h, region: r.id, view, saved: None }));
        update_title(h);
        ShowWindow(h, SW_SHOWNOACTIVATE);
        if full {
            toggle_child_fullscreen(h);
        }
    }
    toast(main, "화면마다 창을 열었습니다 — 닫으면 그 창만 닫힙니다");
}

/// Close the display windows; the session window shows everything again.
unsafe fn gather() {
    for h in child_hwnds() {
        DestroyWindow(h);
    }
    let main = sh().hwnd.load(Relaxed) as HWND;
    if !IsWindowVisible(main).is_positive() {
        ShowWindow(main, SW_SHOW);
    }
    crop_main(None);
}

/// Apply the remembered monitor_mode once the displays (or a manual split) are known:
/// one display by default (monitor_id, else the first), all, or one window each.
unsafe fn apply_monitor_mode() {
    if APPLIED.with(|a| a.get()) {
        return;
    }
    let rs = regions();
    if rs.len() < 2 {
        return; // not known yet, or nothing to divide
    }
    APPLIED.with(|a| a.set(true));
    let (mode, id) = ui(|u| (u.l.opts.monitor_mode, u.l.opts.monitor_id));
    match mode {
        1 => crop_main(None),
        2 => open_windows(),
        _ => crop_main(Some(id.and_then(|id| rs.iter().position(|r| r.id == id)).unwrap_or(0))),
    }
}

/// The framebuffer or its displays changed. Every view follows its display by id, not by
/// position in the list: a display window whose display is gone closes; the session window
/// falls back to the preferred display (monitor_id), then the first, when its own is gone.
unsafe fn refit_children() {
    let rs = regions();
    let find = |id: u32| rs.iter().find(|r| r.id == id);
    if let Some(id) = MAIN_REGION.with(|m| m.get()) {
        let preferred = ui(|u| u.l.opts.monitor_id);
        let r = find(id).or_else(|| preferred.and_then(find)).or(rs.first());
        MAIN_REGION.with(|m| m.set(r.map(|r| r.id)));
        let crop = r.map(|r| r.rect);
        if view().crop != crop {
            let main = sh().hwnd.load(Relaxed) as HWND;
            set_view(View { crop, sx: 0, sy: 0, ..view() });
            update_scroll(main);
            rebuild_cursor(main);
            InvalidateRect(main, null(), 0);
        }
    }
    let mut gone = Vec::new();
    CHILDREN.with(|c| {
        for w in c.borrow_mut().iter_mut() {
            match find(w.region) {
                Some(r) => w.view.crop = Some(r.rect),
                None => gone.push(w.hwnd),
            }
        }
    });
    // (destroyed outside the borrow: WM_DESTROY takes CHILDREN again)
    for h in gone {
        DestroyWindow(h);
    }
    for h in child_hwnds() {
        update_title(h);
        InvalidateRect(h, null(), 0);
    }
}

unsafe extern "system" fn child_proc(h: HWND, m: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    match m {
        WM_PAINT => paint(h),
        WM_ERASEBKGND => return 1,
        WM_SIZE => {
            InvalidateRect(h, null(), 0);
            rebuild_cursor(h);
        }
        WM_SETCURSOR if (lp & 0xffff) as u32 == HTCLIENT => {
            SetCursor(ui(|u| u.cursor).unwrap_or_else(|| LoadCursorW(null_mut(), IDC_ARROW)));
            return 1;
        }
        WM_MOUSEMOVE | WM_LBUTTONDOWN | WM_LBUTTONUP | WM_MBUTTONDOWN | WM_MBUTTONUP | WM_RBUTTONDOWN
        | WM_RBUTTONUP => mouse(h, m, lp),
        WM_MOUSEWHEEL | WM_MOUSEHWHEEL => wheel(m == WM_MOUSEHWHEEL, ((wp >> 16) & 0xffff) as i16 as i32),
        WM_KEYDOWN | WM_SYSKEYDOWN | WM_KEYUP | WM_SYSKEYUP => {
            if !key(h, m, wp, lp) {
                return DefWindowProcW(h, m, wp, lp);
            }
        }
        WM_KILLFOCUS => release_all(),
        WM_CAPTURECHANGED => release_buttons(),
        WM_DPICHANGED => {
            let r = &*(lp as *const RECT);
            SetWindowPos(h, null_mut(), r.left, r.top, r.right - r.left, r.bottom - r.top, SWP_NOZORDER | SWP_NOACTIVATE);
        }
        WM_CLOSE => close(h),
        WM_DESTROY => {
            if GetCapture() == h {
                ReleaseCapture(); // (WM_CAPTURECHANGED lets the buttons go)
            }
            CHILDREN.with(|c| c.borrow_mut().retain(|w| w.hwnd != h));
            // The session window was only hidden for this one: the last view ends the session.
            let main = sh().hwnd.load(Relaxed) as HWND;
            if child_hwnds().is_empty() && !ENDING.with(|e| e.get()) && !IsWindowVisible(main).is_positive() {
                ENDING.with(|e| e.set(true));
                DestroyWindow(main);
            }
        }
        _ => return DefWindowProcW(h, m, wp, lp),
    }
    0
}

// ---------------------------------------------------------------- snapshots

/// Small preview of the remote screen for the home window's address book.
fn save_thumb() {
    let (id, enabled) = ui(|u| {
        u.thumb_done = true;
        (u.l.id.clone(), u.l.thumbnails)
    });
    if !session::managed() || !enabled || id.is_empty() || ui(|u| u.first_frame.is_none()) {
        return;
    }
    // One display reads better on a small card than a strip of several.
    let region = view().crop.or_else(|| sh().screens.lock().unwrap().first().map(|s| [s.x as i32, s.y as i32, s.w as i32, s.h as i32]));
    let img = {
        let fb = sh().fb.lock().unwrap();
        let [x, y, w, hh] = region.unwrap_or([0, 0, fb.w as i32, fb.h as i32]);
        fb.snapshot([x as usize, y as usize, w as usize, hh as usize])
    };
    let png = img.shrink(560).png(6);
    if std::fs::write(crate::store::thumb_path(&id), png).is_ok() {
        session::emit(json!({"ev": "thumb"}));
    }
}

pub fn pictures_dir() -> std::path::PathBuf {
    use windows_sys::Win32::System::Com::CoTaskMemFree;
    use windows_sys::Win32::UI::Shell::{FOLDERID_Pictures, SHGetKnownFolderPath};
    let mut p = null_mut();
    let base = unsafe {
        if SHGetKnownFolderPath(&FOLDERID_Pictures, 0, null_mut(), &mut p) == 0 && !p.is_null() {
            let mut n = 0;
            while *p.add(n) != 0 {
                n += 1;
            }
            let s = String::from_utf16_lossy(std::slice::from_raw_parts(p, n));
            CoTaskMemFree(p as _);
            std::path::PathBuf::from(s)
        } else {
            std::env::temp_dir()
        }
    };
    base.join(crate::APP)
}

fn timestamp() -> String {
    use windows_sys::Win32::System::SystemInformation::GetLocalTime;
    let t = unsafe {
        let mut t = std::mem::zeroed();
        GetLocalTime(&mut t);
        t
    };
    format!("{:04}{:02}{:02}-{:02}{:02}{:02}", t.wYear, t.wMonth, t.wDay, t.wHour, t.wMinute, t.wSecond)
}

unsafe fn screenshot(h: HWND) {
    let img = {
        let fb = sh().fb.lock().unwrap();
        let [x, y, w, hh] = view().crop.unwrap_or([0, 0, fb.w as i32, fb.h as i32]);
        fb.snapshot([x as usize, y as usize, w as usize, hh as usize])
    };
    let dir = pictures_dir();
    let _ = std::fs::create_dir_all(&dir);
    let title: String = ui(|u| u.l.title.clone()).chars().map(|c| if c.is_alphanumeric() || c == '-' || c == '.' { c } else { '_' }).collect();
    let path = dir.join(format!("{title}_{}.png", timestamp()));
    match std::fs::write(&path, img.png(1)) {
        Ok(()) => {
            crate::log!("screenshot {}", path.display());
            toast(h, &format!("스크린샷 저장됨 — 사진\\Itda\\{}", path.file_name().unwrap_or_default().to_string_lossy()));
        }
        Err(e) => toast(h, &format!("스크린샷 저장 실패: {e}")),
    }
}

unsafe fn info(h: HWND) {
    let (title, name, addr, user, started, secure) = ui(|u| (u.l.title.clone(), u.name.clone(), u.addr.clone(), u.l.user.clone(), u.started, u.secure));
    let (w, hh) = sh().size();
    let screens = sh().screens.lock().unwrap().len();
    let secs = started.map_or(0, |t| t.elapsed().as_secs());
    let rtt = sh().rtt_ms().map_or("-".into(), |r| format!("{r:.1} ms"));
    let text = format!(
        "{title}\n\nMac 이름\t{name}\n주소\t{addr}\n보안\t{}\n사용자\t{}\n해상도\t{w} × {hh}\n모니터\t{}\n클립보드\t{}\n응답 시간\t{rtt}\n받은 데이터\t{:.1} MB\n연결 시간\t{:02}:{:02}:{:02}\n\n로그: {}",
        if secure { "SSH 터널 (암호화)" } else { "암호화 안 됨 — 같은 네트워크에서만 쓰세요" },
        if user.is_empty() { "(VNC 암호)" } else { &user },
        if screens == 0 { "정보 없음".to_string() } else { format!("{screens}개") },
        if sh().ext_clip.load(Relaxed) { "확장 (UTF-8, 한글 지원)" } else { "기본" },
        RX_BYTES.load(Relaxed) as f64 / 1e6,
        secs / 3600,
        secs / 60 % 60,
        secs % 60,
        crate::log::path().display(),
    );
    MessageBoxW(h, wide(&text).as_ptr(), wide("연결 정보 — Itda").as_ptr(), MB_ICONINFORMATION);
}

// ---------------------------------------------------------------- clipboard

unsafe fn get_clipboard(h: HWND) -> Option<String> {
    if OpenClipboard(h) == 0 {
        return None;
    }
    let mut out = None;
    let mem = GetClipboardData(CF_UNICODETEXT);
    if !mem.is_null() {
        let p = GlobalLock(mem as _) as *const u16;
        if !p.is_null() {
            let mut n = 0;
            while *p.add(n) != 0 {
                n += 1;
            }
            out = Some(String::from_utf16_lossy(std::slice::from_raw_parts(p, n)));
            GlobalUnlock(mem as _);
        }
    }
    CloseClipboard();
    out
}

unsafe fn set_clipboard(h: HWND, text: &str) {
    let t = text.replace("\r\n", "\n").replace('\n', "\r\n");
    ui(|u| u.last_clip = t.clone());
    let w: Vec<u16> = t.encode_utf16().chain(Some(0)).collect();
    if OpenClipboard(h) == 0 {
        return;
    }
    EmptyClipboard();
    let mem = GlobalAlloc(GMEM_MOVEABLE, w.len() * 2);
    if !mem.is_null() {
        let p = GlobalLock(mem) as *mut u16;
        std::ptr::copy_nonoverlapping(w.as_ptr(), p, w.len());
        GlobalUnlock(mem);
        SetClipboardData(CF_UNICODETEXT, mem as _);
    }
    CloseClipboard();
}

/// Local clipboard changed (or we regained focus): offer it to the Mac if it's new.
unsafe fn clip_out(h: HWND) {
    if !ui(|u| u.l.opts.clipboard) || GetClipboardOwner() == h {
        return;
    }
    let Some(t) = get_clipboard(h) else { return };
    if t.is_empty() || ui(|u| u.last_clip == t) {
        return;
    }
    ui(|u| u.last_clip = t.clone());
    crate::clip::send_local(&t);
}

// ---------------------------------------------------------------- painting

fn sc(h: HWND, v: i32) -> i32 {
    v * unsafe { GetDpiForWindow(h) } as i32 / 96
}

/// Region the stats pill can occupy (top right, clear of the toolbar).
unsafe fn stats_area(h: HWND) -> RECT {
    let (cw, _) = client(h);
    RECT { left: cw - sc(h, 360), top: 0, right: cw, bottom: sc(h, 56) }
}

/// Region a toast can occupy (bottom centre).
unsafe fn toast_area(h: HWND) -> RECT {
    let (cw, ch) = client(h);
    RECT { left: 0.max(cw / 2 - sc(h, 400)), top: ch - sc(h, 90), right: cw.min(cw / 2 + sc(h, 400)), bottom: ch }
}

unsafe fn text_size(hdc: HDC, font: HFONT, s: &str) -> SIZE {
    let w = wide(s);
    let old = SelectObject(hdc, font);
    let mut sz = SIZE { cx: 0, cy: 0 };
    GetTextExtentPoint32W(hdc, w.as_ptr(), w.len() as i32 - 1, &mut sz);
    SelectObject(hdc, old);
    sz
}

unsafe fn round_rect(hdc: HDC, r: &RECT, fill: u32, border: u32, rad: i32) {
    let b = CreateSolidBrush(fill);
    let p = CreatePen(PS_SOLID, 1, border);
    let (ob, op) = (SelectObject(hdc, b), SelectObject(hdc, p));
    RoundRect(hdc, r.left, r.top, r.right, r.bottom, rad, rad);
    SelectObject(hdc, ob);
    SelectObject(hdc, op);
    DeleteObject(b);
    DeleteObject(p);
}

unsafe fn text_at(hdc: HDC, font: HFONT, color: u32, s: &str, r: &RECT, flags: DRAW_TEXT_FORMAT) {
    let w = wide(s);
    let old = SelectObject(hdc, font);
    SetBkMode(hdc, TRANSPARENT as i32);
    SetTextColor(hdc, color);
    let mut rr = *r;
    DrawTextW(hdc, w.as_ptr(), w.len() as i32 - 1, &mut rr, flags | DT_NOPREFIX);
    SelectObject(hdc, old);
}

/// Pills drawn over the image: (rect, text). Computed before the blit so it can skip them.
unsafe fn pills(h: HWND, hdc: HDC) -> Vec<(RECT, String)> {
    let (stats, toast, font) = ui(|u| {
        let st = if u.l.opts.stats && !u.stats_line.is_empty() { Some(u.stats_line.clone()) } else { None };
        (st, u.toast.as_ref().map(|t| t.0.clone()), u.fonts.as_ref().map(|f| f.bold))
    });
    let Some(font) = font else { return Vec::new() };
    let (cw, ch) = client(h);
    let (px, py, m) = (sc(h, 14), sc(h, 7), sc(h, 12));
    let mut out = Vec::new();
    if let Some(s) = stats.filter(|_| sh().status.lock().unwrap().is_empty()) {
        let sz = text_size(hdc, font, &s);
        let r = RECT { left: cw - m - sz.cx - 2 * px, top: m, right: cw - m, bottom: m + sz.cy + 2 * py };
        out.push((r, s));
    }
    if let Some(s) = toast {
        let sz = text_size(hdc, font, &s);
        let w = sz.cx + 2 * px;
        let r = RECT { left: (cw - w) / 2, top: ch - sc(h, 40) - sz.cy - 2 * py, right: (cw + w) / 2, bottom: ch - sc(h, 40) };
        out.push((r, s));
    }
    out
}

unsafe fn paint(h: HWND) {
    let mut ps: PAINTSTRUCT = std::mem::zeroed();
    let hdc = BeginPaint(h, &mut ps);
    draw(h, hdc, ps.rcPaint);
    EndPaint(h, &ps);
}

unsafe fn draw(h: HWND, hdc: HDC, clip: RECT) {
    // Stats and toasts belong to the session window (its timer clears them).
    let pills = if h as isize == sh().hwnd.load(Relaxed) { pills(h, hdc) } else { Vec::new() };
    let disconnected = !sh().status.lock().unwrap().is_empty();
    let saved = SaveDC(hdc);
    if !disconnected {
        // Never paint the image under a pill and then the pill over it: no flicker.
        for (r, _) in &pills {
            ExcludeClipRect(hdc, r.left, r.top, r.right, r.bottom);
        }
    }
    draw_fb(h, hdc, clip);
    RestoreDC(hdc, saved);
    if disconnected {
        draw_card(h, hdc);
    }
    let font = ui(|u| u.fonts.as_ref().map(|f| f.bold));
    if let Some(font) = font {
        for (r, s) in &pills {
            round_rect(hdc, r, 0x0020_2020, 0x0045_4545, sc(h, 14));
            text_at(hdc, font, 0x00f0_f0f0, s, r, DT_CENTER | DT_VCENTER | DT_SINGLELINE);
        }
    }
}

unsafe fn draw_fb(h: HWND, hdc: HDC, clip: RECT) {
    let rects = update_rects(h, hdc, clip);
    FRONT.with(|f| {
        let f = f.borrow();
        for r in rects {
            draw_front(h, hdc, r, &f);
        }
    });
}

/// The parts of `clip` that actually need painting. Several changed rectangles reach
/// WM_PAINT as one bounding box (ps.rcPaint); scaling that whole box would cost as much as
/// the area between them, so each is drawn on its own (up to a dozen, else the box).
unsafe fn update_rects(h: HWND, hdc: HDC, clip: RECT) -> Vec<RECT> {
    let rgn = CreateRectRgn(0, 0, 0, 0);
    let mut out = Vec::new();
    if GetRandomRgn(hdc, rgn, SYSRGN as i32) == 1 {
        let mut o = POINT { x: 0, y: 0 };
        ClientToScreen(h, &mut o); // the system region is in screen coordinates
        OffsetRgn(rgn, -o.x, -o.y);
        let n = GetRegionData(rgn, 0, null_mut());
        let mut buf = vec![0u64; (n as usize).div_ceil(8)];
        if n > 0 && GetRegionData(rgn, n, buf.as_mut_ptr() as *mut RGNDATA) == n {
            let hdr = &*(buf.as_ptr() as *const RGNDATAHEADER);
            let first = (buf.as_ptr() as *const u8).add(hdr.dwSize as usize) as *const RECT;
            for r in std::slice::from_raw_parts(first, hdr.nCount as usize) {
                let r = RECT { left: r.left.max(clip.left), top: r.top.max(clip.top), right: r.right.min(clip.right), bottom: r.bottom.min(clip.bottom) };
                if r.right > r.left && r.bottom > r.top {
                    out.push(r);
                }
            }
        }
    }
    DeleteObject(rgn);
    if out.is_empty() || out.len() > 12 {
        vec![clip]
    } else {
        out
    }
}

unsafe fn draw_front(h: HWND, hdc: HDC, clip: RECT, fb: &Front) {
    if fb.bits.is_null() {
        // Nothing decoded yet: black, not whatever the window held.
        FillRect(hdc, &clip, GetStockObject(BLACK_BRUSH) as HBRUSH);
        return;
    }
    let l = layout(h, fb.w as i32, fb.h as i32);
    let mut rc: RECT = std::mem::zeroed();
    GetClientRect(h, &mut rc);
    let bg = GetStockObject(BLACK_BRUSH) as HBRUSH;
    let (ox, oy, dx1, dy1) = (l.ox, l.oy, l.ox + l.dw, l.oy + l.dh);
    // Letterbox bars only — never paint over the image (no flicker).
    for bar in [
        RECT { left: 0, top: 0, right: rc.right, bottom: oy },
        RECT { left: 0, top: dy1, right: rc.right, bottom: rc.bottom },
        RECT { left: 0, top: oy, right: ox, bottom: dy1 },
        RECT { left: dx1, top: oy, right: rc.right, bottom: dy1 },
    ] {
        if bar.right > bar.left && bar.bottom > bar.top {
            FillRect(hdc, &bar, bg);
        }
    }
    let p = RECT {
        left: clip.left.max(ox),
        top: clip.top.max(oy),
        right: clip.right.min(dx1),
        bottom: clip.bottom.min(dy1),
    };
    if l.fw > 0 && l.fh > 0 && p.right > p.left && p.bottom > p.top {
        let mdc = CreateCompatibleDC(hdc);
        let old = SelectObject(mdc, fb.hbm);
        if l.dw == l.fw && l.dh == l.fh {
            BitBlt(hdc, p.left, p.top, p.right - p.left, p.bottom - p.top, mdc, p.left - ox + l.cx, p.top - oy + l.cy, SRCCOPY);
        } else {
            // Map the update rect back to source pixels (padded so HALFTONE edges get clipped away).
            let pad = (1.0 / l.s).ceil() as i32 + 1;
            let src = |v: i32, o: i32, max: i32, up: bool| {
                let f = (v - o) as f64 / l.s;
                (if up { f.ceil() as i32 + pad } else { f.floor() as i32 - pad }).clamp(0, max)
            };
            let (sx0, sx1) = (src(p.left, ox, l.fw, false), src(p.right, ox, l.fw, true));
            let (sy0, sy1) = (src(p.top, oy, l.fh, false), src(p.bottom, oy, l.fh, true));
            let dst = |v: i32, o: i32| o + (v as f64 * l.s).round() as i32;
            let (x0, x1, y0, y1) = (dst(sx0, ox), dst(sx1, ox), dst(sy0, oy), dst(sy1, oy));
            SetStretchBltMode(hdc, HALFTONE);
            SetBrushOrgEx(hdc, 0, 0, null_mut());
            StretchBlt(hdc, x0, y0, x1 - x0, y1 - y0, mdc, sx0 + l.cx, sy0 + l.cy, sx1 - sx0, sy1 - sy0, SRCCOPY);
        }
        SelectObject(mdc, old);
        DeleteDC(mdc);
    }
    GdiFlush(); // finish reading the DIB before on_fb writes into it again
}

/// Link lost: dim the last frame and show what's happening, with a way out.
unsafe fn draw_card(h: HWND, hdc: HDC) {
    let (cw, ch) = client(h);
    // dim
    let mdc = CreateCompatibleDC(hdc);
    let bi = dib_info(1, 1);
    let mut bits = null_mut();
    let bmp = CreateDIBSection(null_mut(), &bi, DIB_RGB_COLORS, &mut bits, null_mut(), 0);
    let old = SelectObject(mdc, bmp);
    let bf = BLENDFUNCTION { BlendOp: 0, BlendFlags: 0, SourceConstantAlpha: 150, AlphaFormat: 0 };
    AlphaBlend(hdc, 0, 0, cw, ch, mdc, 0, 0, 1, 1, bf);
    SelectObject(mdc, old);
    DeleteObject(bmp);
    DeleteDC(mdc);

    let Some((text, bold, title, icon)) = ui(|u| u.fonts.as_ref().map(|f| (f.text, f.bold, f.title, f.icon))) else { return };
    let (w, hh) = (sc(h, 400), sc(h, 176));
    let card = RECT { left: (cw - w) / 2, top: (ch - hh) / 2, right: (cw + w) / 2, bottom: (ch + hh) / 2 };
    round_rect(hdc, &card, 0x002b_2b2b, 0x0045_4545, sc(h, 16));
    let pad = sc(h, 24);
    let ir = RECT { left: card.left + pad, top: card.top + pad, right: card.left + pad + sc(h, 36), bottom: card.top + pad + sc(h, 36) };
    text_at(hdc, icon, 0x0000_b9fc, "\u{E7BA}", &ir, DT_LEFT | DT_TOP | DT_SINGLELINE);
    let tx = ir.right + sc(h, 12);
    let tr = RECT { left: tx, top: card.top + pad, right: card.right - pad, bottom: card.top + pad + sc(h, 26) };
    text_at(hdc, title, 0x00ff_ffff, "Mac과의 연결이 끊겼습니다", &tr, DT_LEFT | DT_TOP | DT_SINGLELINE);
    let st = sh().status.lock().unwrap().clone();
    let sr = RECT { left: tx, top: tr.bottom + sc(h, 6), right: card.right - pad, bottom: tr.bottom + sc(h, 50) };
    let msg = format!("{st}\n네트워크가 돌아오면 자동으로 이어집니다.");
    text_at(hdc, text, 0x00c8_c8c8, &msg, &sr, DT_LEFT | DT_TOP | DT_WORDBREAK);
    let (bw, bh) = (sc(h, 96), sc(h, 32));
    let btn = RECT { left: card.right - pad - bw, top: card.bottom - pad - bh, right: card.right - pad, bottom: card.bottom - pad };
    round_rect(hdc, &btn, 0x003d_3d3d, 0x0055_5555, sc(h, 8));
    text_at(hdc, bold, 0x00ff_ffff, "연결 닫기", &btn, DT_CENTER | DT_VCENTER | DT_SINGLELINE);
    ui(|u| u.close_btn = [btn.left, btn.top, btn.right, btn.bottom]);
}

unsafe fn toast(h: HWND, s: &str) {
    ui(|u| u.toast = Some((s.to_string(), Instant::now())));
    InvalidateRect(h, &toast_area(h), 0);
}

unsafe fn on_fb(h: HWND) {
    sh().ack();
    ui(|u| {
        if u.first_frame.is_none() {
            u.first_frame = Some(Instant::now());
        }
    });
    // Copy what changed into the window's own copy, holding the decoder up only for that.
    let (fresh, changed, fw, fh) = {
        let mut fb = sh().fb.lock().unwrap();
        if fb.dirty.is_empty() {
            return;
        }
        let Some((fresh, changed, fw, fh)) = FRONT.with(|f| {
            let mut f = f.borrow_mut();
            let fresh = f.fit(fb.w, fb.h);
            if f.bits.is_null() {
                return None; // couldn't make it: keep the damage for the next try
            }
            let list = if fresh || fb.dirty.is_all() { vec![(0, 0, fb.w, fb.h)] } else { fb.dirty.rects().to_vec() };
            let dst = std::slice::from_raw_parts_mut(f.bits, f.w * f.h);
            let src = fb.pixels();
            for &(x, y, w, hh) in &list {
                for row in y..y + hh {
                    let a = row * f.w + x;
                    dst[a..a + w].copy_from_slice(&src[a..a + w]);
                }
            }
            Some((fresh, list, f.w, f.h))
        }) else {
            return;
        };
        fb.dirty.clear();
        (fresh, changed, fw, fh)
    };
    // The session window and every display window paint from the copy just made.
    let windows: Vec<HWND> = std::iter::once(h).chain(child_hwnds()).collect();
    if fresh {
        // A new size (or the first frame): the layout moved, letterbox bars included.
        for &w in &windows {
            InvalidateRect(w, null(), 0);
            UpdateWindow(w);
        }
        return;
    }
    // Each changed rectangle on its own: a clock in one corner and the pointer in another
    // don't repaint everything between them. Laid out by the size just copied, which is
    // what draw_front paints; a window whose display isn't touched isn't repainted.
    for &win in &windows {
        let l = layout(win, fw as i32, fh as i32);
        let (cw, ch) = client(win);
        for &(x, y, w, hh) in &changed {
            let (x0, x1, y0, y1) = (x as i32 - l.cx, (x + w) as i32 - l.cx, y as i32 - l.cy, (y + hh) as i32 - l.cy);
            if x1 <= 0 || y1 <= 0 || x0 >= l.fw || y0 >= l.fh {
                continue; // outside this window's display
            }
            let r = RECT {
                left: l.ox + (x0 as f64 * l.s).floor() as i32 - 2,
                top: l.oy + (y0 as f64 * l.s).floor() as i32 - 2,
                right: l.ox + (x1 as f64 * l.s).ceil() as i32 + 2,
                bottom: l.oy + (y1 as f64 * l.s).ceil() as i32 + 2,
            };
            if r.right > 0 && r.bottom > 0 && r.left < cw && r.top < ch {
                InvalidateRect(win, &r, 0);
            }
        }
        UpdateWindow(win); // paint now rather than waiting for an idle queue
    }
}

// ---------------------------------------------------------------- input

unsafe fn to_fb(h: HWND, x: i32, y: i32) -> (u16, u16) {
    let l = cur_layout(h);
    let c = |v: i32, o: i32, max: i32, base: i32| ((((v - o) as f64 / l.s) as i32).clamp(0, (max - 1).max(0)) + base) as u16;
    (c(x, l.ox, l.fw, l.cx), c(y, l.oy, l.fh, l.cy))
}

unsafe fn mouse(h: HWND, m: u32, lp: LPARAM) {
    let (x, y) = ((lp & 0xffff) as i16 as i32, ((lp >> 16) & 0xffff) as i16 as i32);
    if m == WM_MOUSEMOVE && h as isize == sh().hwnd.load(Relaxed) {
        toolbar::parent_mouse(y);
    }
    if !sh().status.lock().unwrap().is_empty() {
        // Disconnected: only the card's close button is live.
        let [l, t, r, b] = ui(|u| u.close_btn);
        if m == WM_LBUTTONUP && x >= l && x < r && y >= t && y < b {
            end_session(h);
        }
        return;
    }
    if VIEW_ONLY.load(Relaxed) {
        return;
    }
    let pos = to_fb(h, x, y);
    let (bit, down) = match m {
        WM_LBUTTONDOWN => (1, true),
        WM_LBUTTONUP => (1, false),
        WM_MBUTTONDOWN => (2, true),
        WM_MBUTTONUP => (2, false),
        WM_RBUTTONDOWN => (4, true),
        WM_RBUTTONUP => (4, false),
        _ => (0, false),
    };
    let b = ui(|u| {
        if down {
            u.buttons |= bit;
        } else {
            u.buttons &= !bit;
        }
        u.pos = pos;
        u.buttons
    });
    if bit != 0 {
        if b != 0 {
            SetCapture(h);
        } else {
            ReleaseCapture();
        }
    }
    sh().pointer(b, pos.0, pos.1);
}

fn wheel(horizontal: bool, delta: i32) {
    if VIEW_ONLY.load(Relaxed) {
        return;
    }
    let apple = crate::rfb::APPLE.load(Relaxed);
    let (b, pos, clicks) = ui(|u| {
        let acc = if horizontal { &mut u.wheel.1 } else { &mut u.wheel.0 };
        *acc += delta;
        let n = *acc / 120;
        *acc -= n * 120;
        if n == 0 || !apple {
            return (u.buttons, u.pos, n);
        }
        // A Mac scrolls a short line per click: send Windows' lines (or characters) per notch,
        // times the chosen speed, so a notch goes about as far as it does here.
        let speed = match u.l.mac_scroll.as_str() {
            "normal" => 1.0,
            "faster" => 3.0,
            _ => 2.0,
        };
        // Notches in quick succession speed up, as a Mac's own wheel does: up to ×3 after
        // a few fast ones in the same direction.
        let dir = n.signum() * if horizontal { 2 } else { 1 };
        let now = Instant::now();
        let quick = u.wheel_last.is_some_and(|(t, d)| d == dir && now.duration_since(t) < Duration::from_millis(150));
        u.wheel_run = if quick && u.l.scroll_accel { (u.wheel_run + 1).min(4) } else { 0 };
        u.wheel_last = Some((now, dir));
        let accel = 1.0 + 0.5 * u.wheel_run as f64;
        let per_notch = (wheel_lines(horizontal) as f64 * speed * accel).round() as i32;
        (u.buttons, u.pos, n * per_notch.max(1))
    });
    // X11 buttons 4/5 = up/down, 6/7 = left/right.
    let bit = match (horizontal, clicks > 0) {
        (false, true) => 8,
        (false, false) => 16,
        (true, true) => 64,
        (true, false) => 32,
    };
    for _ in 0..clicks.abs() {
        sh().pointer(b | bit, pos.0, pos.1);
        sh().pointer(b, pos.0, pos.1);
    }
}

/// Windows' "lines (characters) to scroll per notch" setting, 3 by default; "one screen at a
/// time" counts as 10.
fn wheel_lines(horizontal: bool) -> i32 {
    let mut n: u32 = 3;
    let what = if horizontal { SPI_GETWHEELSCROLLCHARS } else { SPI_GETWHEELSCROLLLINES };
    unsafe { SystemParametersInfoW(what, 0, &mut n as *mut u32 as _, 0) };
    if n == u32::MAX { 10 } else { n.clamp(1, 20) as i32 }
}

fn down(vk: u16) -> bool {
    unsafe { GetKeyState(vk as i32) < 0 }
}

/// Returns false when Windows should handle the key instead.
unsafe fn key(h: HWND, m: u32, wp: WPARAM, lp: LPARAM) -> bool {
    let vk = wp as u32;
    let press = m == WM_KEYDOWN || m == WM_SYSKEYDOWN;
    let scan = ((lp >> 16) & 0xff) as u32;
    let ext = (lp >> 24) & 1 != 0;
    let id = if scan != 0 { scan | (ext as u32) << 8 } else { 0x10000 | vk };

    // Local hotkeys: Ctrl+Alt+<key> (or Ctrl+Shift+<key>), never forwarded.
    let shift_style = ui(|u| u.l.hotkey == "ctrl_shift");
    let prefix = down(VK_CONTROL) && if shift_style { down(VK_SHIFT) && !down(VK_MENU) } else { down(VK_MENU) && !down(VK_SHIFT) };
    if press && prefix {
        let cmd = match vk {
            0x0d => Some(M_FULL),
            0x30 => Some(M_FIT),
            0x31 => Some(M_ACTUAL),
            0x53 => Some(M_STATS),
            0x56 => Some(M_VIEWONLY),
            0x52 => Some(M_RECONNECT),
            0x54 => Some(M_PIN),
            0x50 => Some(M_SHOT),
            0x49 => Some(M_INFO),
            0x4d => {
                release_all();
                show_menu(h);
                return true;
            }
            _ => None,
        };
        if let Some(c) = cmd {
            release_all(); // the Mac saw Ctrl (+Alt) go down; don't leave them stuck
            command(h, c);
            return true;
        }
    }
    if vk == VK_F4 as u32 && down(VK_MENU) && !down(VK_CONTROL) {
        return false; // Alt+F4 closes the viewer
    }
    if vk == VK_LWIN as u32 || vk == VK_RWIN as u32 || VIEW_ONLY.load(Relaxed) {
        return true; // Win keys go through the low-level hook
    }
    if vk == VK_HANGUL as u32 {
        // Korean/English toggle → the Mac's input-source shortcut.
        if press {
            let combo: &[u32] = if ui(|u| u.l.opts.hangul_caps) { &[0xffe5] } else { &[keys::XK_CONTROL_L, 0x20] };
            combo.iter().for_each(|&k| sh().key(true, k));
            combo.iter().rev().for_each(|&k| sh().key(false, k));
        }
        return true;
    }
    if press {
        if let Some(sym) = keys::keysym(vk, scan, ext, SWAP.load(Relaxed)) {
            ui(|u| u.keys.insert(id, sym));
            sh().key(true, sym);
        }
    } else if let Some(sym) = ui(|u| u.keys.remove(&id)) {
        sh().key(false, sym);
    }
    true
}

/// Grabs the Win keys while our window is focused so they reach the Mac as Command.
unsafe extern "system" fn kb_hook(code: i32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    if code == HC_ACTION as i32 && !VIEW_ONLY.load(Relaxed) {
        let k = &*(lp as *const KBDLLHOOKSTRUCT);
        let hwnd = sh().hwnd.load(Relaxed) as HWND;
        let fg = GetForegroundWindow();
        let ours = fg == hwnd || CHILDREN.with(|c| c.try_borrow().is_ok_and(|c| c.iter().any(|w| w.hwnd == fg)));
        if (k.vkCode == VK_LWIN as u32 || k.vkCode == VK_RWIN as u32) && ours {
            let press = wp as u32 == WM_KEYDOWN || wp as u32 == WM_SYSKEYDOWN;
            let sym = keys::win_sym(k.vkCode == VK_RWIN as u32, SWAP.load(Relaxed));
            let id = 0x20000 | k.vkCode;
            UI.with(|u| {
                if let Ok(mut u) = u.try_borrow_mut() {
                    if press {
                        u.keys.insert(id, sym);
                    } else {
                        u.keys.remove(&id);
                    }
                }
            });
            sh().key(press, sym);
            return 1;
        }
    }
    CallNextHookEx(null_mut(), code, wp, lp)
}

/// Focus lost / mode change: release everything so no modifier stays stuck on the Mac.
/// Mouse capture went away (window closed, another window took it) with a button still
/// down: let it go on the remote too, or it stays held there. Keys are left alone.
fn release_buttons() {
    let (b, pos) = ui(|u| (std::mem::take(&mut u.buttons), u.pos));
    if b != 0 {
        sh().pointer(0, pos.0, pos.1);
    }
}

fn release_all() {
    let (keys, b, pos) = ui(|u| {
        let b = u.buttons;
        u.buttons = 0;
        (u.keys.drain().map(|(_, s)| s).collect::<Vec<_>>(), b, u.pos)
    });
    for s in keys {
        sh().key(false, s);
    }
    if b != 0 {
        sh().pointer(0, pos.0, pos.1);
    }
}

unsafe fn toggle_fullscreen(h: HWND) {
    if h as isize != sh().hwnd.load(Relaxed) {
        return toggle_child_fullscreen(h);
    }
    if let Some((style, wp)) = ui(|u| u.saved.take()) {
        SetWindowLongPtrW(h, GWL_STYLE, style);
        SetWindowPlacement(h, &wp);
        SetWindowPos(h, null_mut(), 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_NOZORDER | SWP_FRAMECHANGED);
    } else {
        let style = GetWindowLongPtrW(h, GWL_STYLE);
        let mut wp: WINDOWPLACEMENT = std::mem::zeroed();
        wp.length = size_of::<WINDOWPLACEMENT>() as u32;
        GetWindowPlacement(h, &mut wp);
        let mut mi: MONITORINFO = std::mem::zeroed();
        mi.cbSize = size_of::<MONITORINFO>() as u32;
        GetMonitorInfoW(MonitorFromWindow(h, MONITOR_DEFAULTTONEAREST), &mut mi);
        ui(|u| u.saved = Some((style, wp)));
        SetWindowLongPtrW(h, GWL_STYLE, style & !(WS_OVERLAPPEDWINDOW as isize));
        let r = mi.rcMonitor;
        SetWindowPos(h, HWND_TOP, r.left, r.top, r.right - r.left, r.bottom - r.top, SWP_FRAMECHANGED);
    }
    let full = ui(|u| u.saved.is_some());
    ui(|u| u.l.opts.fullscreen = full);
    emit_opts();
}

/// A display window's full screen: its own placement, not saved in the host's options.
unsafe fn toggle_child_fullscreen(h: HWND) {
    let saved = CHILDREN.with(|c| c.borrow_mut().iter_mut().find(|w| w.hwnd == h).and_then(|w| w.saved.take()));
    if let Some((style, wp)) = saved {
        SetWindowLongPtrW(h, GWL_STYLE, style);
        SetWindowPlacement(h, &wp);
        SetWindowPos(h, null_mut(), 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_NOZORDER | SWP_FRAMECHANGED);
        remember_child_fullscreen();
        return;
    }
    let style = GetWindowLongPtrW(h, GWL_STYLE);
    let mut wp: WINDOWPLACEMENT = std::mem::zeroed();
    wp.length = size_of::<WINDOWPLACEMENT>() as u32;
    GetWindowPlacement(h, &mut wp);
    let mut mi: MONITORINFO = std::mem::zeroed();
    mi.cbSize = size_of::<MONITORINFO>() as u32;
    GetMonitorInfoW(MonitorFromWindow(h, MONITOR_DEFAULTTONEAREST), &mut mi);
    CHILDREN.with(|c| {
        if let Some(w) = c.borrow_mut().iter_mut().find(|w| w.hwnd == h) {
            w.saved = Some((style, wp));
        }
    });
    SetWindowLongPtrW(h, GWL_STYLE, style & !(WS_OVERLAPPEDWINDOW as isize));
    let r = mi.rcMonitor;
    SetWindowPos(h, HWND_TOP, r.left, r.top, r.right - r.left, r.bottom - r.top, SWP_FRAMECHANGED);
    remember_child_fullscreen();
}

/// monitor_fullscreen follows the display windows: on while any of them is full screen.
fn remember_child_fullscreen() {
    let any = CHILDREN.with(|c| c.borrow().iter().any(|w| w.saved.is_some()));
    if ui(|u| std::mem::replace(&mut u.l.opts.monitor_fullscreen, any)) != any {
        emit_opts();
    }
}

// ---------------------------------------------------------------- cursor

/// (Re)build the local cursor from the server's image at the current display scale.
unsafe fn rebuild_cursor(h: HWND) {
    let s = cur_layout(h).s;
    let new = ui(|u| {
        let c = u.cursor_img.as_ref()?;
        Some(if c.w == 0 || c.h == 0 { null_mut() } else { make_hcursor(c, s) })
    });
    let Some(new) = new else { return };
    if let Some(old) = ui(|u| u.cursor.replace(new)) {
        if !old.is_null() {
            DestroyCursor(old);
        }
    }
    let mut pt = POINT { x: 0, y: 0 };
    GetCursorPos(&mut pt);
    if WindowFromPoint(pt) == h && GetCapture().is_null() {
        let mut rc: RECT = std::mem::zeroed();
        GetClientRect(h, &mut rc);
        ScreenToClient(h, &mut pt);
        if pt.x >= 0 && pt.y >= 0 && pt.x < rc.right && pt.y < rc.bottom {
            SetCursor(new);
        }
    } else if !GetCapture().is_null() {
        SetCursor(new);
    }
}

unsafe fn make_hcursor(c: &Cursor, s: f64) -> HCURSOR {
    let w = ((c.w as f64 * s).round() as usize).max(1);
    let hgt = ((c.h as f64 * s).round() as usize).max(1);
    let bi = dib_info(w as i32, hgt as i32);
    let mut bits = null_mut();
    let color = CreateDIBSection(null_mut(), &bi, DIB_RGB_COLORS, &mut bits, null_mut(), 0);
    let dst = std::slice::from_raw_parts_mut(bits as *mut u32, w * hgt);
    for y in 0..hgt {
        let sy = (y * c.h / hgt).min(c.h - 1);
        for x in 0..w {
            let sx = (x * c.w / w).min(c.w - 1);
            dst[y * w + x] = c.px[sy * c.w + sx];
        }
    }
    let zeros = vec![0u8; (w + 15) / 16 * 2 * hgt];
    let mask = CreateBitmap(w as i32, hgt as i32, 1, 1, zeros.as_ptr() as _);
    let ii = ICONINFO {
        fIcon: 0,
        xHotspot: ((c.hx as f64 * s) as u32).min(w as u32 - 1),
        yHotspot: ((c.hy as f64 * s) as u32).min(hgt as u32 - 1),
        hbmMask: mask,
        hbmColor: color,
    };
    let hc = CreateIconIndirect(&ii);
    DeleteObject(color);
    DeleteObject(mask);
    hc
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Manual splits cover the framebuffer exactly, in reading order, odd sizes included.
    #[test]
    fn manual_splits() {
        assert_eq!(split(0, 1920, 1080), None);
        assert_eq!(split(1, 3840, 1080).unwrap(), [[0, 0, 1920, 1080], [1920, 0, 1920, 1080]]);
        assert_eq!(split(2, 1920, 2160).unwrap(), [[0, 0, 1920, 1080], [0, 1080, 1920, 1080]]);
        assert_eq!(split(3, 5761, 1080).unwrap(), [[0, 0, 1920, 1080], [1920, 0, 1920, 1080], [3840, 0, 1921, 1080]]);
        let q = split(4, 3841, 2161).unwrap();
        assert_eq!(q.len(), 4);
        assert_eq!(q[0], [0, 0, 1920, 1080]);
        assert_eq!(q[3], [1920, 1080, 1921, 1081]);
        let area: i32 = q.iter().map(|r| r[2] * r[3]).sum();
        assert_eq!(area, 3841 * 2161);
        assert_eq!(split(3, 2, 10), None, "too small to split");
        assert_eq!(LAYOUTS.len(), 5);
    }
}
