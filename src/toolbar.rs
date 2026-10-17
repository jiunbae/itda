//! In-session toolbar: a child window at the top centre of the viewer. Collapsed it is a
//! small pull-down handle; hovering it (or the top edge) expands the buttons. Auto mode
//! collapses again shortly after the mouse leaves; pinned stays open; hidden shows nothing.

use crate::ui::{self, wide, Look};
use std::cell::RefCell;
use std::ptr::{null, null_mut};
use windows_sys::Win32::Foundation::*;
use windows_sys::Win32::Graphics::Gdi::*;
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::UI::Controls::*;
use windows_sys::Win32::UI::HiDpi::GetDpiForWindow;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::*;
use windows_sys::Win32::UI::WindowsAndMessaging::*;

pub enum Item {
    Button(usize, &'static str),
    Sep,
}

const TIMER_COLLAPSE: usize = 1;
const BG: u32 = 0x002b_2b2b;
const HOVER: u32 = 0x003d_3d3d;
const PRESSED: u32 = 0x0047_4747;
const DANGER: u32 = 0x001c_2bc4; // BGR of #c42b1c
const FG: u32 = 0x00f0_f0f0;
const ACCENT: u32 = 0x00c2_d133; // BGR of #33d1c2 (Itda teal)
const BORDER: u32 = 0x0045_4545;

#[derive(Default)]
struct Tb {
    hwnd: HWND,
    parent: HWND,
    tip: HWND,
    items: Vec<(Item, RECT)>,
    expanded: bool,
    /// "auto" | "pinned" | "hidden"
    mode: String,
    hover: Option<usize>,
    pressed: Option<usize>,
    tracking: bool,
    /// A dropdown menu is open: don't collapse underneath it.
    menu_open: bool,
    icons: HFONT,
    dpi: i32,
}

thread_local! {
    static TB: RefCell<Tb> = RefCell::new(Tb::default());
}

fn tb<R>(f: impl FnOnce(&mut Tb) -> R) -> R {
    TB.with(|t| f(&mut t.borrow_mut()))
}

fn icon_font(dpi: i32) -> HFONT {
    // Windows 11 ships Segoe Fluent Icons; Windows 10 has the same code points in MDL2.
    let face = if std::path::Path::new(r"C:\Windows\Fonts\SegoeIcons.ttf").exists() { "Segoe Fluent Icons" } else { "Segoe MDL2 Assets" };
    unsafe { CreateFontW(-(16 * dpi / 96), 0, 0, 0, 400, 0, 0, 0, 1, 0, 0, 5, 0, wide(face).as_ptr()) }
}

pub fn create(parent: HWND, items: Vec<Item>, mode: &str) -> HWND {
    unsafe {
        let hinst = GetModuleHandleW(null());
        let cls = wide("itda_tb");
        let wc = WNDCLASSEXW {
            cbSize: size_of::<WNDCLASSEXW>() as u32,
            lpfnWndProc: Some(proc_),
            hInstance: hinst,
            hCursor: LoadCursorW(null_mut(), IDC_ARROW),
            lpszClassName: cls.as_ptr(),
            ..std::mem::zeroed()
        };
        RegisterClassExW(&wc);
        let h = CreateWindowExW(0, cls.as_ptr(), null(), WS_CHILD | WS_CLIPSIBLINGS, 0, 0, 10, 10, parent, null_mut(), hinst, null());
        let tip = CreateWindowExW(WS_EX_TOPMOST, wide("tooltips_class32").as_ptr(), null(), WS_POPUP | TTS_NOPREFIX | TTS_ALWAYSTIP,
            0, 0, 0, 0, h, null_mut(), hinst, null());
        let dpi = GetDpiForWindow(parent) as i32;
        tb(|t| {
            t.hwnd = h;
            t.parent = parent;
            t.tip = tip;
            t.items = items.into_iter().map(|i| (i, RECT { left: 0, top: 0, right: 0, bottom: 0 })).collect();
            t.mode = mode.to_string();
            t.expanded = mode != "hidden";
            t.dpi = dpi;
            t.icons = icon_font(dpi);
        });
        add_tips();
        layout();
        if mode == "auto" {
            SetTimer(h, TIMER_COLLAPSE, 2500, None); // show once at start so people find it
        }
        h
    }
}

unsafe fn add_tips() {
    let (h, tip, tips) = tb(|t| {
        let tips: Vec<(usize, &'static str)> =
            t.items.iter().enumerate().filter_map(|(i, (it, _))| if let Item::Button(_, s) = it { Some((i, *s)) } else { None }).collect();
        (t.hwnd, t.tip, tips)
    });
    for (i, text) in tips {
        let w = wide(text);
        let mut ti: TTTOOLINFOW = std::mem::zeroed();
        ti.cbSize = size_of::<TTTOOLINFOW>() as u32;
        ti.uFlags = TTF_SUBCLASS;
        ti.hwnd = h;
        ti.uId = i;
        ti.lpszText = w.as_ptr() as *mut u16;
        SendMessageW(tip, TTM_ADDTOOLW, 0, &ti as *const _ as isize);
    }
}

fn s(dpi: i32, v: i32) -> i32 {
    v * dpi / 96
}

/// Toolbar size and button rects at the current DPI (expanded geometry).
fn measure(t: &mut Tb) -> (i32, i32) {
    let d = t.dpi;
    let (bw, bh, pad, sep) = (s(d, 40), s(d, 34), s(d, 5), s(d, 11));
    let mut x = pad;
    for (it, r) in t.items.iter_mut() {
        let w = if matches!(it, Item::Sep) { sep } else { bw };
        *r = RECT { left: x, top: pad, right: x + w, bottom: pad + bh };
        x += w;
    }
    (x + pad, bh + 2 * pad)
}

fn handle_size(d: i32) -> (i32, i32) {
    (s(d, 64), s(d, 12))
}

/// Re-place the toolbar in the parent (after resize, mode or expand changes).
pub fn layout() {
    unsafe {
        let Some((h, parent, expanded, mode, d, (w, hh), tip, rects)) = TB.with(|t| {
            let mut t = t.borrow_mut();
            if t.hwnd.is_null() {
                return None;
            }
            let size = measure(&mut t);
            let rects: Vec<RECT> = t.items.iter().map(|(_, r)| *r).collect();
            Some((t.hwnd, t.parent, t.expanded, t.mode.clone(), t.dpi, size, t.tip, rects))
        }) else {
            return;
        };
        if mode == "hidden" {
            ShowWindow(h, SW_HIDE);
            return;
        }
        let mut rc: RECT = std::mem::zeroed();
        GetClientRect(parent, &mut rc);
        let (w, hh) = if expanded { (w, hh) } else { handle_size(d) };
        let x = (rc.right - w) / 2;
        let r = s(d, if expanded { 10 } else { 8 });
        SetWindowPos(h, HWND_TOP, x, 0, w, hh, SWP_NOACTIVATE | SWP_SHOWWINDOW);
        // Rounded bottom corners; the top edge sits flush with the window edge.
        SetWindowRgn(h, CreateRoundRectRgn(0, -r, w + 1, hh + 1, 2 * r, 2 * r), 1);
        for (i, rect) in rects.iter().enumerate() {
            let mut ti: TTTOOLINFOW = std::mem::zeroed();
            ti.cbSize = size_of::<TTTOOLINFOW>() as u32;
            ti.hwnd = h;
            ti.uId = i;
            ti.rect = if expanded { *rect } else { RECT { left: 0, top: 0, right: 0, bottom: 0 } };
            SendMessageW(tip, TTM_NEWTOOLRECTW, 0, &ti as *const _ as isize);
        }
        InvalidateRect(h, null(), 0);
    }
}

pub fn set_mode(mode: &str) {
    let h = tb(|t| {
        t.mode = mode.to_string();
        t.expanded = mode == "pinned";
        t.hwnd
    });
    unsafe { KillTimer(h, TIMER_COLLAPSE) };
    layout();
}

pub fn mode() -> String {
    tb(|t| t.mode.clone())
}

/// Repaint (toggle states changed).
pub fn refresh() {
    let h = tb(|t| t.hwnd);
    if !h.is_null() {
        unsafe { InvalidateRect(h, null(), 0) };
    }
}

pub fn dpi_changed(dpi: i32) {
    tb(|t| {
        unsafe { DeleteObject(t.icons) };
        t.dpi = dpi;
        t.icons = icon_font(dpi);
    });
    layout();
}

/// Mouse moved in the viewer: the very top edge opens the toolbar (handy in fullscreen).
pub fn parent_mouse(y: i32) {
    let (h, expanded, mode) = tb(|t| (t.hwnd, t.expanded, t.mode.clone()));
    if !h.is_null() && !expanded && mode == "auto" && y <= 1 {
        expand(true);
    }
}

fn expand(on: bool) {
    let changed = tb(|t| {
        let c = t.expanded != on;
        t.expanded = on;
        if !on {
            t.hover = None;
        }
        c
    });
    if changed {
        layout();
    }
}

unsafe fn paint(h: HWND) {
    let mut ps: PAINTSTRUCT = std::mem::zeroed();
    let hdc = BeginPaint(h, &mut ps);
    let mut rc: RECT = std::mem::zeroed();
    GetClientRect(h, &mut rc);
    let mdc = CreateCompatibleDC(hdc);
    let bmp = CreateCompatibleBitmap(hdc, rc.right, rc.bottom);
    let old = SelectObject(mdc, bmp);
    draw(mdc, rc);
    BitBlt(hdc, 0, 0, rc.right, rc.bottom, mdc, 0, 0, SRCCOPY);
    SelectObject(mdc, old);
    DeleteObject(bmp);
    DeleteDC(mdc);
    EndPaint(h, &ps);
}

/// Also used by tests/screenshots through WM_PRINTCLIENT.
unsafe fn draw(hdc: HDC, rc: RECT) {
    let fill = |r: &RECT, c: u32| {
        let b = CreateSolidBrush(c);
        FillRect(hdc, r, b);
        DeleteObject(b);
    };
    let round = |r: &RECT, c: u32, rad: i32| {
        let b = CreateSolidBrush(c);
        let p = CreatePen(PS_SOLID, 1, c);
        let (ob, op) = (SelectObject(hdc, b), SelectObject(hdc, p));
        RoundRect(hdc, r.left, r.top, r.right, r.bottom, rad, rad);
        SelectObject(hdc, ob);
        SelectObject(hdc, op);
        DeleteObject(b);
        DeleteObject(p);
    };
    fill(&rc, BG);
    let (expanded, d, icons, hover, pressed) = tb(|t| (t.expanded, t.dpi, t.icons, t.hover, t.pressed));
    // hairline border along the bottom/sides
    let pen = CreatePen(PS_SOLID, 1, BORDER);
    let op = SelectObject(hdc, pen);
    let ob = SelectObject(hdc, GetStockObject(NULL_BRUSH));
    let r = s(d, if expanded { 10 } else { 8 });
    RoundRect(hdc, 0, -r, rc.right, rc.bottom, 2 * r, 2 * r);
    SelectObject(hdc, ob);
    SelectObject(hdc, op);
    DeleteObject(pen);

    if !expanded {
        // grab bar
        let (w, hh) = (s(d, 24), s(d, 3));
        let bar = RECT { left: (rc.right - w) / 2, top: (rc.bottom - hh) / 2, right: (rc.right + w) / 2, bottom: (rc.bottom + hh) / 2 };
        round(&bar, 0x00a0_a0a0, hh);
        return;
    }
    SetBkMode(hdc, TRANSPARENT as i32);
    let old_font = SelectObject(hdc, icons);
    let items: Vec<(Option<usize>, RECT)> =
        TB.with(|t| t.borrow().items.iter().map(|(it, r)| (if let Item::Button(id, _) = it { Some(*id) } else { None }, *r)).collect());
    for (i, (id, r)) in items.into_iter().enumerate() {
        let Some(id) = id else {
            let x = (r.left + r.right) / 2;
            fill(&RECT { left: x, top: r.top + s(d, 8), right: x + 1, bottom: r.bottom - s(d, 8) }, BORDER);
            continue;
        };
        let Look { glyph, on, danger, enabled } = ui::look(id);
        if enabled && (hover == Some(i) || pressed == Some(i)) {
            let c = if danger { DANGER } else if pressed == Some(i) { PRESSED } else { HOVER };
            round(&r, c, s(d, 8));
        }
        if on {
            let (w, y) = (s(d, 14), r.bottom - s(d, 4));
            round(&RECT { left: (r.left + r.right - w) / 2, top: y, right: (r.left + r.right + w) / 2, bottom: y + s(d, 3) }, ACCENT, s(d, 3));
        }
        SetTextColor(hdc, if !enabled { 0x0070_7070 } else if on { ACCENT } else { FG });
        let g = wide(&glyph.to_string());
        let mut rr = r;
        DrawTextW(hdc, g.as_ptr(), 1, &mut rr, DT_CENTER | DT_VCENTER | DT_SINGLELINE | DT_NOPREFIX);
    }
    SelectObject(hdc, old_font);
}

fn hit(x: i32, y: i32) -> Option<usize> {
    tb(|t| {
        if !t.expanded {
            return None;
        }
        t.items.iter().position(|(it, r)| matches!(it, Item::Button(..)) && x >= r.left && x < r.right && y >= r.top && y < r.bottom)
    })
}

unsafe extern "system" fn proc_(h: HWND, m: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    let (x, y) = ((lp & 0xffff) as i16 as i32, ((lp >> 16) & 0xffff) as i16 as i32);
    match m {
        WM_PAINT => paint(h),
        WM_PRINTCLIENT => {
            let mut rc: RECT = std::mem::zeroed();
            GetClientRect(h, &mut rc);
            draw(wp as HDC, rc);
        }
        WM_ERASEBKGND => return 1,
        WM_MOUSEACTIVATE => return MA_NOACTIVATE as isize,
        WM_MOUSEMOVE => {
            KillTimer(h, TIMER_COLLAPSE);
            if !tb(|t| t.tracking) {
                let mut tme = TRACKMOUSEEVENT { cbSize: size_of::<TRACKMOUSEEVENT>() as u32, dwFlags: TME_LEAVE, hwndTrack: h, dwHoverTime: 0 };
                TrackMouseEvent(&mut tme);
                tb(|t| t.tracking = true);
            }
            if !tb(|t| t.expanded) {
                expand(true);
                return 0;
            }
            let hv = hit(x, y);
            if tb(|t| std::mem::replace(&mut t.hover, hv) != hv) {
                InvalidateRect(h, null(), 0);
            }
        }
        WM_MOUSELEAVE => {
            let auto = tb(|t| {
                t.tracking = false;
                t.hover = None;
                t.mode == "auto" && !t.menu_open
            });
            InvalidateRect(h, null(), 0);
            if auto {
                SetTimer(h, TIMER_COLLAPSE, 600, None);
            }
        }
        WM_TIMER if wp == TIMER_COLLAPSE => {
            KillTimer(h, TIMER_COLLAPSE);
            let (mode, menu) = tb(|t| (t.mode.clone(), t.menu_open));
            if mode == "auto" && !menu {
                expand(false);
            }
        }
        WM_LBUTTONDOWN => {
            let p = hit(x, y);
            tb(|t| t.pressed = p);
            if p.is_some() {
                SetCapture(h);
            }
            InvalidateRect(h, null(), 0);
        }
        WM_LBUTTONUP => {
            let p = tb(|t| t.pressed.take());
            ReleaseCapture();
            InvalidateRect(h, null(), 0);
            if let Some(i) = p.filter(|&i| hit(x, y) == Some(i)) {
                let (id, r, parent) = tb(|t| {
                    let (it, r) = &t.items[i];
                    (if let Item::Button(id, _) = it { *id } else { 0 }, *r, t.parent)
                });
                // Anchor for dropdown menus: bottom-left of the button, in screen coordinates.
                let mut pt = POINT { x: r.left, y: r.bottom + 2 };
                ClientToScreen(h, &mut pt);
                tb(|t| t.menu_open = true);
                ui::toolbar_click(parent, id, pt);
                tb(|t| t.menu_open = false);
                SetFocus(parent);
                // The mouse may have left while a menu was open.
                let mut cur = POINT { x: 0, y: 0 };
                GetCursorPos(&mut cur);
                if WindowFromPoint(cur) != h && tb(|t| t.mode == "auto") {
                    SetTimer(h, TIMER_COLLAPSE, 600, None);
                }
            }
        }
        WM_DESTROY => tb(|t| unsafe {
            DeleteObject(t.icons);
            t.hwnd = null_mut();
        }),
        _ => return DefWindowProcW(h, m, wp, lp),
    }
    0
}
