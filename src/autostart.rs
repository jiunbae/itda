//! Start Itda when the person at this computer logs in: on Windows a per-user Run entry
//! (`itda.exe --login`, which starts in the notification area), on macOS the app itself as a
//! login item (SMAppService; shown under System Settings > General > Login Items). The system
//! holds the state: it can also be changed there, so Itda reads it back rather than trusting
//! its own setting.

/// This launch came from the login entry (Windows): stay in the notification area.
pub static AT_LOGIN: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum State {
    Off,
    On,
    /// Registered, but macOS waits for the person to allow it in System Settings.
    NeedsApproval,
}

impl State {
    pub fn name(self) -> &'static str {
        match self {
            State::Off => "off",
            State::On => "on",
            State::NeedsApproval => "approval",
        }
    }

    /// Registered (whether or not it may run yet).
    pub fn on(self) -> bool {
        self != State::Off
    }
}

/// Whether Itda starts at login now.
pub fn state() -> State {
    imp::state()
}

/// Turn starting at login on or off; the state that resulted.
pub fn set(on: bool) -> Result<State, String> {
    imp::set(on)?;
    Ok(state())
}

/// Where the person can allow or remove the login item themselves.
pub fn open_settings() {
    imp::open_settings()
}

/// Ask for `want` with `set`. On failure the state stays what the system says (`now`), so
/// the setting never claims a change that didn't happen.
pub fn apply(want: bool, set: impl FnOnce(bool) -> Result<State, String>, now: impl FnOnce() -> State) -> (State, Option<String>) {
    match set(want) {
        Ok(s) => (s, None),
        Err(e) => (now(), Some(e)),
    }
}

#[cfg(windows)]
mod imp {
    use super::State;
    use std::ptr::null_mut;
    use windows_sys::Win32::System::Registry::{RegDeleteKeyValueW, RegGetValueW, RegSetKeyValueW, HKEY_CURRENT_USER, REG_SZ, RRF_RT_REG_SZ};

    const RUN: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
    const NAME: &str = "Itda";

    fn wide(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(Some(0)).collect()
    }

    pub fn state() -> State {
        let (key, name) = (wide(RUN), wide(NAME));
        let mut n = 0u32;
        let r = unsafe { RegGetValueW(HKEY_CURRENT_USER, key.as_ptr(), name.as_ptr(), RRF_RT_REG_SZ, null_mut(), null_mut(), &mut n) };
        if r == 0 { State::On } else { State::Off }
    }

    pub fn set(on: bool) -> Result<(), String> {
        let (key, name) = (wide(RUN), wide(NAME));
        let r = if on {
            let exe = std::env::current_exe().map_err(|e| e.to_string())?;
            let cmd = wide(&format!("\"{}\" --login", exe.display()));
            unsafe { RegSetKeyValueW(HKEY_CURRENT_USER, key.as_ptr(), name.as_ptr(), REG_SZ, cmd.as_ptr().cast(), (cmd.len() * 2) as u32) }
        } else {
            match unsafe { RegDeleteKeyValueW(HKEY_CURRENT_USER, key.as_ptr(), name.as_ptr()) } {
                2 => 0, // ERROR_FILE_NOT_FOUND: already off
                r => r,
            }
        };
        if r == 0 { Ok(()) } else { Err(std::io::Error::from_raw_os_error(r as i32).to_string()) }
    }

    pub fn open_settings() {
        let _ = std::process::Command::new("explorer").arg("ms-settings:startupapps").spawn();
    }
}

#[cfg(target_os = "macos")]
mod imp {
    use super::State;
    use objc2::msg_send;
    use objc2::rc::Retained;
    use objc2::runtime::{AnyClass, AnyObject, Bool};
    use objc2_foundation::NSError;

    #[link(name = "ServiceManagement", kind = "framework")]
    unsafe extern "C" {}

    fn class() -> Option<&'static AnyClass> {
        AnyClass::get(c"SMAppService")
    }

    fn service() -> Option<Retained<AnyObject>> {
        class().map(|cls| unsafe { msg_send![cls, mainAppService] })
    }

    pub fn state() -> State {
        // SMAppServiceStatus: 0 not registered, 1 enabled, 2 requires approval, 3 not found.
        match service().map(|s| unsafe { msg_send![&*s, status] }) {
            Some(1isize) => State::On,
            Some(2) => State::NeedsApproval,
            _ => State::Off,
        }
    }

    pub fn set(on: bool) -> Result<(), String> {
        let Some(service) = service() else { return Err("이 macOS에서는 로그인 항목을 등록할 수 없습니다".into()) };
        if on == state().on() {
            return Ok(());
        }
        unsafe {
            let mut err: *mut NSError = std::ptr::null_mut();
            let ok: Bool = if on {
                msg_send![&*service, registerAndReturnError: &mut err]
            } else {
                msg_send![&*service, unregisterAndReturnError: &mut err]
            };
            if ok.as_bool() {
                return Ok(());
            }
            Err(err.as_ref().map_or_else(|| "알 수 없는 오류".into(), |e| e.localizedDescription().to_string()))
        }
    }

    pub fn open_settings() {
        if let Some(cls) = class() {
            let _: () = unsafe { msg_send![cls, openSystemSettingsLoginItems] };
        }
    }
}

#[cfg(not(any(windows, target_os = "macos")))]
mod imp {
    use super::State;
    pub fn state() -> State {
        State::Off
    }
    pub fn set(_on: bool) -> Result<(), String> {
        Err("이 시스템에서는 지원하지 않습니다".into())
    }
    pub fn open_settings() {}
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_failed_change_keeps_what_the_system_has() {
        // Turning off fails: still on, and said so.
        let (s, e) = apply(false, |_| Err("denied".into()), || State::On);
        assert_eq!((s, e.as_deref()), (State::On, Some("denied")));
        // Turning on fails: still off.
        let (s, e) = apply(true, |_| Err("denied".into()), || State::Off);
        assert_eq!((s, e.is_some()), (State::Off, true));
        // Registered but waiting for approval: on, though not running yet.
        let (s, e) = apply(true, |_| Ok(State::NeedsApproval), || unreachable!());
        assert!(s.on() && s.name() == "approval" && e.is_none());
        let (s, _) = apply(false, |_| Ok(State::Off), || unreachable!());
        assert!(!s.on());
    }

    /// The SMAppService calls are well-formed: reading the status of a test binary (no
    /// bundle) must answer, not crash.
    #[cfg(target_os = "macos")]
    #[test]
    fn mac_status_answers() {
        let _ = state();
    }
}
