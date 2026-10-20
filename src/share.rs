//! "Share this PC": what Windows provides to the VNC server in server.rs, so other devices
//! (Itda on another PC, the web viewer, macOS Screen Sharing, any VNC client) can see and
//! control this PC.
//!
//! One capture of the whole virtual desktop (all monitors; Desktop Duplication, else GDI)
//! shared by all viewers, SendInput for mouse and keyboard, the clipboard, and the
//! on-screen "allow?" prompt.

pub use crate::server::{kick_all, open_local, running, start, stop, viewers, Config, ON_CHANGE};
use std::collections::HashMap;
use std::net::SocketAddr;
use std::ptr::null_mut;
use std::sync::atomic::{AtomicBool, Ordering::Relaxed};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use windows_sys::Win32::Foundation::*;
use windows_sys::Win32::Graphics::Gdi::*;
use windows_sys::Win32::System::DataExchange::*;
use windows_sys::Win32::System::Memory::{GlobalAlloc, GlobalLock, GlobalUnlock, GMEM_MOVEABLE};
use windows_sys::Win32::UI::Input::KeyboardAndMouse::*;
use windows_sys::Win32::UI::WindowsAndMessaging::*;

pub(crate) const KIND: &str = "PC";
pub(crate) const MDNS_TXT: &[(&str, &str)] = &[("itda", "1"), ("os", "windows")];

pub(crate) fn name() -> String {
    std::env::var("COMPUTERNAME").unwrap_or_else(|_| "Windows".into())
}

pub(crate) fn ready() -> Result<(), String> {
    Ok(())
}

pub(crate) fn clipboard_seq() -> u64 {
    unsafe { GetClipboardSequenceNumber() as u64 }
}

/// Media Foundation's H.264 encoder, when this PC has it (else ZRLE).
pub(crate) fn video(w: usize, h: usize) -> Option<crate::server::Video> {
    crate::h264_win::video(w, h)
}

pub(crate) fn ask_user(peer: &SocketAddr) -> bool {
    let text = format!(
        "{}에서 이 PC의 화면을 보고 조작하려고 합니다.\n\n허용할까요?\n\n(허용하면 연결이 끝날 때까지 화면과 입력이 공유됩니다. 알림 영역의 Itda에서 언제든 끊을 수 있습니다.)",
        peer.ip()
    );
    let t = crate::ui::wide(&text);
    let title = crate::ui::wide("Itda — 화면 공유 요청");
    unsafe { MessageBoxW(null_mut(), t.as_ptr(), title.as_ptr(), MB_YESNO | MB_ICONQUESTION | MB_TOPMOST | MB_SETFOREGROUND | MB_SYSTEMMODAL) == IDYES }
}

// ---------------------------------------------------------------- capture

/// This PC's displays in desktop coordinates: (id, left, top, right, bottom). The id is a
/// hash of the display's device name, so it stays with the display when the layout changes.
pub(crate) fn monitors() -> Vec<(u32, i32, i32, i32, i32)> {
    unsafe extern "system" fn each(m: HMONITOR, _: HDC, _: *mut RECT, data: LPARAM) -> BOOL {
        let list = &mut *(data as *mut Vec<(u32, i32, i32, i32, i32)>);
        let mut info: MONITORINFOEXW = std::mem::zeroed();
        info.monitorInfo.cbSize = size_of::<MONITORINFOEXW>() as u32;
        if GetMonitorInfoW(m, &mut info as *mut MONITORINFOEXW as *mut MONITORINFO) != 0 {
            let r = info.monitorInfo.rcMonitor;
            let id = info.szDevice.iter().take_while(|&&c| c != 0).fold(0x811c_9dc5u32, |h, &c| (h ^ c as u32).wrapping_mul(0x0100_0193));
            list.push((id, r.left, r.top, r.right, r.bottom));
        }
        1
    }
    let mut list: Vec<(u32, i32, i32, i32, i32)> = Vec::new();
    unsafe { EnumDisplayMonitors(null_mut(), std::ptr::null(), Some(each), &mut list as *mut _ as LPARAM) };
    list
}

/// The one capture all viewers share: a display can be duplicated only once per process,
/// and one capture is cheaper anyway. Each viewer diffs it against its own last frame.
/// Released (duplication included) when the last viewer leaves.
pub(crate) fn capture() -> Arc<Mutex<Capture>> {
    static SHARED: Mutex<std::sync::Weak<Mutex<Capture>>> = Mutex::new(std::sync::Weak::new());
    let mut g = SHARED.lock().unwrap();
    g.upgrade().unwrap_or_else(|| {
        let c = Arc::new(Mutex::new(Capture::new()));
        *g = Arc::downgrade(&c);
        c
    })
}

pub(crate) struct Capture {
    pub(crate) x: i32,
    pub(crate) y: i32,
    pub(crate) w: usize,
    pub(crate) h: usize,
    mdc: HDC,
    bmp: HBITMAP,
    bits: *mut u32,
    /// Desktop Duplication, when it works; else (and while it's being retried) GDI.
    dup: Option<crate::dxgi::Dup>,
    dup_retry: Instant,
    /// The pixels the drawn-in cursor covers, to put back before the next frame:
    /// duplication only refreshes what changed on the screen.
    under: Option<([i32; 4], Vec<u32>)>,
    gdi_logged: bool,
}

// GDI objects and the D3D context are used by one viewer's thread at a time (the mutex).
unsafe impl Send for Capture {}

impl Capture {
    fn new() -> Capture {
        let mut c = Capture { x: 0, y: 0, w: 0, h: 0, mdc: null_mut(), bmp: null_mut(), bits: null_mut(), dup: None, dup_retry: Instant::now(), under: None, gdi_logged: false };
        c.fit();
        c
    }

    /// (Re)size to the virtual desktop (all monitors).
    pub(crate) fn fit(&mut self) {
        let (x, y, w, h) = unsafe {
            (
                GetSystemMetrics(SM_XVIRTUALSCREEN),
                GetSystemMetrics(SM_YVIRTUALSCREEN),
                GetSystemMetrics(SM_CXVIRTUALSCREEN).max(1) as usize,
                GetSystemMetrics(SM_CYVIRTUALSCREEN).max(1) as usize,
            )
        };
        if (x, y, w, h) == (self.x, self.y, self.w, self.h) && !self.bmp.is_null() {
            return;
        }
        self.free();
        unsafe {
            let screen = GetDC(null_mut());
            self.mdc = CreateCompatibleDC(screen);
            let bi = crate::fb::dib_info(w as i32, h as i32);
            let mut bits = null_mut();
            self.bmp = CreateDIBSection(screen, &bi, DIB_RGB_COLORS, &mut bits, null_mut(), 0);
            self.bits = bits as *mut u32;
            SelectObject(self.mdc, self.bmp);
            ReleaseDC(null_mut(), screen);
        }
        (self.x, self.y, self.w, self.h) = (x, y, w, h);
        // New layout: displays moved, and the new buffer starts empty.
        self.dup = None;
        self.dup_retry = Instant::now();
        self.under = None;
    }

    pub(crate) fn grab(&mut self) -> (usize, usize, &[u32]) {
        self.fit();
        unsafe {
            let (w, h) = (self.w, self.h);
            let px = std::slice::from_raw_parts_mut(self.bits, w * h);
            if let Some(([x, y, cw, ch], saved)) = self.under.take() {
                for r in 0..ch as usize {
                    let o = (y as usize + r) * w + x as usize;
                    px[o..o + cw as usize].copy_from_slice(&saved[r * cw as usize..(r + 1) * cw as usize]);
                }
            }
            // Duplication delivers changes only, so a new one starts from a whole GDI frame
            // (its own first frame is whole too).
            let mut gdi = self.dup.is_none();
            if self.dup.is_none() && Instant::now() >= self.dup_retry {
                match crate::dxgi::Dup::new() {
                    Ok(d) => {
                        crate::log!("capture: desktop duplication");
                        self.dup = Some(d);
                    }
                    Err(e) => {
                        if !self.gdi_logged {
                            crate::log!("capture: GDI ({e})");
                            self.gdi_logged = true;
                        }
                        self.dup_retry = Instant::now() + Duration::from_secs(2);
                    }
                }
            }
            if !gdi {
                if let Some(d) = &mut self.dup {
                    if d.update(px, w, h, self.x, self.y).is_err() {
                        crate::log!("capture: desktop duplication lost");
                        self.gdi_logged = false;
                        self.dup = None;
                        self.dup_retry = Instant::now() + Duration::from_millis(500);
                        gdi = true;
                    }
                }
            }
            if gdi {
                let screen = GetDC(null_mut());
                BitBlt(self.mdc, 0, 0, w as i32, h as i32, screen, self.x, self.y, SRCCOPY | CAPTUREBLT);
                ReleaseDC(null_mut(), screen);
                GdiFlush();
                for p in px.iter_mut() {
                    *p &= 0x00ff_ffff;
                }
            }
            // Neither capture includes the cursor: draw it in, keeping what it covers.
            let mut ci: CURSORINFO = std::mem::zeroed();
            ci.cbSize = size_of::<CURSORINFO>() as u32;
            if GetCursorInfo(&mut ci) != 0 && ci.flags & CURSOR_SHOWING != 0 {
                let mut ii: ICONINFO = std::mem::zeroed();
                if GetIconInfo(ci.hCursor, &mut ii) != 0 {
                    let mut bm: BITMAP = std::mem::zeroed();
                    let (bw, bh) = if GetObjectW(ii.hbmMask, size_of::<BITMAP>() as i32, &mut bm as *mut BITMAP as _) != 0 {
                        (bm.bmWidth, if ii.hbmColor.is_null() { bm.bmHeight / 2 } else { bm.bmHeight })
                    } else {
                        (64, 64)
                    };
                    let (cx, cy) = (ci.ptScreenPos.x - self.x - ii.xHotspot as i32, ci.ptScreenPos.y - self.y - ii.yHotspot as i32);
                    let (x0, y0) = (cx.max(0), cy.max(0));
                    let (x1, y1) = ((cx + bw).min(w as i32), (cy + bh).min(h as i32));
                    if x0 < x1 && y0 < y1 {
                        let (rw, rh) = ((x1 - x0) as usize, (y1 - y0) as usize);
                        let mut saved = Vec::with_capacity(rw * rh);
                        for r in 0..rh {
                            let o = (y0 as usize + r) * w + x0 as usize;
                            saved.extend_from_slice(&px[o..o + rw]);
                        }
                        DrawIconEx(self.mdc, cx, cy, ci.hCursor, 0, 0, 0, null_mut(), DI_NORMAL);
                        GdiFlush();
                        for r in 0..rh {
                            let o = (y0 as usize + r) * w + x0 as usize;
                            for p in &mut px[o..o + rw] {
                                *p &= 0x00ff_ffff;
                            }
                        }
                        self.under = Some(([x0, y0, rw as i32, rh as i32], saved));
                    }
                    if !ii.hbmMask.is_null() {
                        DeleteObject(ii.hbmMask);
                    }
                    if !ii.hbmColor.is_null() {
                        DeleteObject(ii.hbmColor);
                    }
                }
            }
            (w, h, px)
        }
    }

    fn free(&mut self) {
        unsafe {
            if !self.mdc.is_null() {
                DeleteDC(self.mdc);
            }
            if !self.bmp.is_null() {
                DeleteObject(self.bmp);
            }
        }
        self.mdc = null_mut();
        self.bmp = null_mut();
    }
}

impl Drop for Capture {
    fn drop(&mut self) {
        self.free();
    }
}

// ---------------------------------------------------------------- input

/// Longest a modifier may stay down without any other key, and how long a silent viewer
/// may hold one (see `read_client`).
pub(crate) const MOD_HOLD_MAX: Duration = Duration::from_secs(15);
pub(crate) const MOD_QUIET: Duration = Duration::from_secs(4);

pub(crate) struct Input {
    cmd_as_ctrl: bool,
    view_only: bool,
    buttons: u8,
    /// keysym → what we pressed for it, so the release matches.
    down: HashMap<u32, Pressed>,
    /// Last key event, for releasing forgotten modifiers.
    pub(crate) last_key: Instant,
}

#[derive(Clone, Copy)]
enum Pressed {
    Vk(u16, bool),
    Unicode(u16),
}

fn send(inputs: &[INPUT]) {
    let n = unsafe { SendInput(inputs.len() as u32, inputs.as_ptr(), size_of::<INPUT>() as i32) };
    if n == 0 {
        // Blocked: no input desktop (locked/secure screen) or an elevated window has focus.
        static WARNED: AtomicBool = AtomicBool::new(false);
        if !WARNED.swap(true, Relaxed) {
            crate::log!("share: input blocked by Windows (error {}); is the screen locked or an admin window focused?", unsafe { GetLastError() });
        }
    }
}

fn key_input(vk: u16, scan: u16, flags: KEYBD_EVENT_FLAGS) -> INPUT {
    let mut i: INPUT = unsafe { std::mem::zeroed() };
    i.r#type = INPUT_KEYBOARD;
    i.Anonymous.ki = KEYBDINPUT { wVk: vk, wScan: scan, dwFlags: flags, time: 0, dwExtraInfo: 0 };
    i
}

impl Input {
    pub(crate) fn new(cmd_as_ctrl: bool, view_only: bool) -> Input {
        Input { cmd_as_ctrl, view_only, buttons: 0, down: HashMap::new(), last_key: Instant::now() }
    }

    fn is_modifier(p: &Pressed) -> bool {
        matches!(p, Pressed::Vk(vk, _) if [VK_LCONTROL, VK_RCONTROL, VK_LMENU, VK_RMENU, VK_LWIN, VK_RWIN, VK_LSHIFT, VK_RSHIFT].contains(vk))
    }

    pub(crate) fn release_modifiers(&mut self, why: &str) {
        let held: Vec<u32> = self.down.iter().filter(|(_, p)| Self::is_modifier(p)).map(|(s, _)| *s).collect();
        for s in held {
            if let Some(p) = self.down.remove(&s) {
                Self::release(p);
            }
        }
        crate::log!("share: released modifier keys ({why})");
    }

    fn special(&self, sym: u32) -> Option<(u16, bool)> {
        // (virtual key, extended)
        Some(match sym {
            0xff08 => (VK_BACK, false),
            0xff09 => (VK_TAB, false),
            0xff0d => (VK_RETURN, false),
            0xff8d => (VK_RETURN, true),
            0xff13 => (VK_PAUSE, false),
            0xff14 => (VK_SCROLL, false),
            0xff1b => (VK_ESCAPE, false),
            0xffff => (VK_DELETE, true),
            0xff50 => (VK_HOME, true),
            0xff51 => (VK_LEFT, true),
            0xff52 => (VK_UP, true),
            0xff53 => (VK_RIGHT, true),
            0xff54 => (VK_DOWN, true),
            0xff55 => (VK_PRIOR, true),
            0xff56 => (VK_NEXT, true),
            0xff57 => (VK_END, true),
            0xff61 => (VK_SNAPSHOT, false),
            0xff63 => (VK_INSERT, true),
            0xff67 => (VK_APPS, true),
            0xff7f => (VK_NUMLOCK, false),
            0xffaa => (VK_MULTIPLY, false),
            0xffab => (VK_ADD, false),
            0xffad => (VK_SUBTRACT, false),
            0xffae => (VK_DECIMAL, false),
            0xffaf => (VK_DIVIDE, true),
            0xffb0..=0xffb9 => (VK_NUMPAD0 + (sym - 0xffb0) as u16, false),
            0xffbe..=0xffc9 => (VK_F1 + (sym - 0xffbe) as u16, false),
            0xffe1 => (VK_LSHIFT, false),
            0xffe2 => (VK_RSHIFT, false),
            0xffe3 => (VK_LCONTROL, false),
            0xffe4 => (VK_RCONTROL, true),
            0xffe5 => (VK_CAPITAL, false),
            0xffe7 | 0xffe9 => (VK_LMENU, false),
            0xffe8 | 0xffea => (VK_RMENU, true),
            0xffeb | 0xffec if self.cmd_as_ctrl => (VK_LCONTROL, false),
            0xffeb => (VK_LWIN, true),
            0xffec => (VK_RWIN, true),
            _ => return None,
        })
    }

    pub(crate) fn modifiers_held(&self) -> bool {
        self.down.values().any(|p| matches!(p, Pressed::Vk(vk, _) if [VK_LCONTROL, VK_RCONTROL, VK_LMENU, VK_RMENU, VK_LWIN, VK_RWIN].contains(vk)))
    }

    pub(crate) fn key(&mut self, down: bool, sym: u32) {
        if self.view_only {
            return;
        }
        self.last_key = Instant::now();
        if !down {
            if let Some(p) = self.down.remove(&sym) {
                Self::release(p);
            }
            return;
        }
        let pressed = if let Some((vk, ext)) = self.special(sym) {
            Pressed::Vk(vk, ext)
        } else {
            let cp = if sym >= 0x0100_0000 { sym & 0x00ff_ffff } else { sym };
            let Some(ch) = char::from_u32(cp) else { return };
            if self.modifiers_held() && ch.is_ascii_alphanumeric() {
                // Shortcuts (Ctrl+C, Alt+F…) need real keys, not Unicode characters.
                Pressed::Vk(ch.to_ascii_uppercase() as u16, false)
            } else if self.modifiers_held() {
                let vk = unsafe { VkKeyScanW(ch as u16) };
                if vk == -1 {
                    return;
                }
                Pressed::Vk((vk & 0xff) as u16, false)
            } else {
                let mut b = [0u16; 2];
                let s = ch.encode_utf16(&mut b);
                if s.len() != 1 {
                    // Outside the BMP: send the surrogate pair as one keystroke.
                    send(&[key_input(0, s[0], KEYEVENTF_UNICODE), key_input(0, s[1], KEYEVENTF_UNICODE)]);
                    return;
                }
                Pressed::Unicode(s[0])
            }
        };
        match pressed {
            Pressed::Vk(vk, ext) => send(&[key_input(vk, 0, if ext { KEYEVENTF_EXTENDEDKEY } else { 0 })]),
            Pressed::Unicode(u) => send(&[key_input(0, u, KEYEVENTF_UNICODE)]),
        }
        self.down.insert(sym, pressed);
    }

    fn release(p: Pressed) {
        match p {
            Pressed::Vk(vk, ext) => send(&[key_input(vk, 0, KEYEVENTF_KEYUP | if ext { KEYEVENTF_EXTENDEDKEY } else { 0 })]),
            Pressed::Unicode(u) => send(&[key_input(0, u, KEYEVENTF_UNICODE | KEYEVENTF_KEYUP)]),
        }
    }

    pub(crate) fn pointer(&mut self, mask: u8, x: u16, y: u16) {
        if self.view_only {
            return;
        }
        let (vw, vh) = unsafe { (GetSystemMetrics(SM_CXVIRTUALSCREEN).max(2) as i64, GetSystemMetrics(SM_CYVIRTUALSCREEN).max(2) as i64) };
        // Windows maps back with (n * size) >> 16; rounding up lands on exactly this pixel.
        let norm = |v: u16, size: i64| ((v as i64 * 65536 + size - 1) / size).min(65535) as i32;
        let mut i: INPUT = unsafe { std::mem::zeroed() };
        i.r#type = INPUT_MOUSE;
        let mut flags = MOUSEEVENTF_MOVE | MOUSEEVENTF_ABSOLUTE | MOUSEEVENTF_VIRTUALDESK;
        let mut data: i32 = 0;
        let changed = mask ^ self.buttons;
        let press = |bit: u8| changed & bit != 0 && mask & bit != 0;
        let lift = |bit: u8| changed & bit != 0 && mask & bit == 0;
        if press(1) { flags |= MOUSEEVENTF_LEFTDOWN; }
        if lift(1) { flags |= MOUSEEVENTF_LEFTUP; }
        if press(2) { flags |= MOUSEEVENTF_MIDDLEDOWN; }
        if lift(2) { flags |= MOUSEEVENTF_MIDDLEUP; }
        if press(4) { flags |= MOUSEEVENTF_RIGHTDOWN; }
        if lift(4) { flags |= MOUSEEVENTF_RIGHTUP; }
        if press(8) { flags |= MOUSEEVENTF_WHEEL; data = 120; }
        if press(16) { flags |= MOUSEEVENTF_WHEEL; data = -120; }
        if press(32) { flags |= MOUSEEVENTF_HWHEEL; data = -120; }
        if press(64) { flags |= MOUSEEVENTF_HWHEEL; data = 120; }
        self.buttons = mask;
        i.Anonymous.mi = MOUSEINPUT {
            dx: norm(x, vw),
            dy: norm(y, vh),
            mouseData: data as u32,
            dwFlags: flags,
            time: 0,
            dwExtraInfo: 0,
        };
        send(&[i]);
    }

    pub(crate) fn release_all(&mut self) {
        for (_, p) in self.down.drain() {
            Self::release(p);
        }
        if self.buttons & 7 != 0 {
            self.pointer(0, 0, 0);
        }
    }
}

// ---------------------------------------------------------------- clipboard

const CF_UNICODETEXT: u32 = 13;

pub(crate) fn clipboard_get() -> Option<String> {
    unsafe {
        if OpenClipboard(null_mut()) == 0 {
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
        out.filter(|s| !s.is_empty())
    }
}

pub(crate) fn clipboard_set(text: &str) {
    let t = text.replace("\r\n", "\n").replace('\n', "\r\n");
    let w: Vec<u16> = t.encode_utf16().chain(Some(0)).collect();
    unsafe {
        if OpenClipboard(null_mut()) == 0 {
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
}

