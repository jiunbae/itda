//! Windows virtual keys -> X11 keysyms. macOS maps Super->Command, Alt->Option.

use windows_sys::Win32::UI::Input::KeyboardAndMouse::{GetKeyboardLayout, GetKeyboardState, ToUnicodeEx};

pub const XK_CONTROL_L: u32 = 0xffe3;
pub const XK_CONTROL_R: u32 = 0xffe4;
pub const XK_SUPER_L: u32 = 0xffeb;
pub const XK_SUPER_R: u32 = 0xffec;

/// Ctrl and Win keysyms, optionally swapped so Ctrl acts as Command.
pub fn ctrl_sym(right: bool, swap: bool) -> u32 {
    match (swap, right) {
        (false, false) => XK_CONTROL_L,
        (false, true) => XK_CONTROL_R,
        (true, false) => XK_SUPER_L,
        (true, true) => XK_SUPER_R,
    }
}
pub fn win_sym(right: bool, swap: bool) -> u32 {
    ctrl_sym(right, !swap)
}

pub fn keysym(vk: u32, scan: u32, ext: bool, swap: bool) -> Option<u32> {
    special(vk, scan, ext, swap).or_else(|| char_sym(vk, scan))
}

fn special(vk: u32, scan: u32, ext: bool, swap: bool) -> Option<u32> {
    Some(match vk {
        0x08 => 0xff08,                                  // Backspace
        0x09 => 0xff09,                                  // Tab
        0x0d => if ext { 0xff8d } else { 0xff0d },       // Enter / KP_Enter
        0x10 => if scan == 0x36 { 0xffe2 } else { 0xffe1 }, // Shift
        0x11 => ctrl_sym(ext, swap),
        0x12 => if ext { 0xffea } else { 0xffe9 },       // Alt -> Option
        0x13 => 0xff13,                                  // Pause
        0x14 => 0xffe5,                                  // Caps Lock
        0x1b => 0xff1b,                                  // Esc
        0x20 => 0x0020,                                  // Space
        0x21 => 0xff55,                                  // PgUp
        0x22 => 0xff56,                                  // PgDn
        0x23 => 0xff57,                                  // End
        0x24 => 0xff50,                                  // Home
        0x25 => 0xff51,                                  // Left
        0x26 => 0xff52,                                  // Up
        0x27 => 0xff53,                                  // Right
        0x28 => 0xff54,                                  // Down
        0x2c => 0xff61,                                  // PrintScreen
        0x2d => 0xff63,                                  // Insert
        0x2e => 0xffff,                                  // Delete
        0x5d => 0xff67,                                  // Menu
        0x60..=0x69 => 0xffb0 + (vk - 0x60),             // KP_0..9
        0x6a => 0xffaa,
        0x6b => 0xffab,
        0x6c => 0xffac,
        0x6d => 0xffad,
        0x6e => 0xffae,
        0x6f => 0xffaf,
        0x70..=0x87 => 0xffbe + (vk - 0x70),             // F1..F24
        0x90 => 0xff7f,                                  // NumLock
        0x91 => 0xff14,                                  // ScrollLock
        0xa0 => 0xffe1,
        0xa1 => 0xffe2,
        0xa2 => ctrl_sym(false, swap),
        0xa3 => ctrl_sym(true, swap),
        0xa4 => 0xffe9,
        0xa5 => 0xffea,
        _ => return None,
    })
}

/// Printable key: the character it produces with Shift applied but Ctrl/Alt/CapsLock
/// ignored (the Mac applies its own modifiers and Caps Lock state).
fn char_sym(vk: u32, scan: u32) -> Option<u32> {
    let mut st = [0u8; 256];
    let mut buf = [0u16; 4];
    let n = unsafe {
        GetKeyboardState(st.as_mut_ptr());
        for k in [0x11, 0x12, 0xa2, 0xa3, 0xa4, 0xa5] {
            st[k] = 0;
        }
        st[0x14] = 0;
        // flag 4: don't touch the kernel's dead-key state
        ToUnicodeEx(vk, scan, st.as_ptr(), buf.as_mut_ptr(), 4, 4, GetKeyboardLayout(0))
    };
    if n != 1 {
        return None;
    }
    let c = buf[0] as u32;
    Some(if (0x20..0x7f).contains(&c) || (0xa0..0x100).contains(&c) { c } else { 0x0100_0000 | c })
}
