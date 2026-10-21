//! Start Itda when the person at this computer logs in: on Windows a per-user Run entry
//! (`itda.exe --login`, which starts in the notification area), on macOS the app itself as a
//! login item (SMAppService; shown under System Settings > General > Login Items).

/// This launch came from the login entry (Windows): stay in the notification area.
pub static AT_LOGIN: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Turn starting at login on or off.
pub fn set(on: bool) -> Result<(), String> {
    imp::set(on)
}

#[cfg(windows)]
mod imp {
    use windows_sys::Win32::System::Registry::{RegDeleteKeyValueW, RegSetKeyValueW, HKEY_CURRENT_USER, REG_SZ};

    const RUN: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
    const NAME: &str = "Itda";

    fn wide(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(Some(0)).collect()
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
}

#[cfg(target_os = "macos")]
mod imp {
    use objc2::rc::Retained;
    use objc2::runtime::{AnyClass, AnyObject, Bool};
    use objc2::msg_send;
    use objc2_foundation::NSError;

    #[link(name = "ServiceManagement", kind = "framework")]
    unsafe extern "C" {}

    pub fn set(on: bool) -> Result<(), String> {
        let Some(cls) = AnyClass::get(c"SMAppService") else { return Err("이 macOS에서는 로그인 항목을 등록할 수 없습니다".into()) };
        unsafe {
            let service: Retained<AnyObject> = msg_send![cls, mainAppService];
            // SMAppServiceStatus: 0 not registered, 1 enabled, 2 requires approval, 3 not found.
            let status: isize = msg_send![&*service, status];
            if on == (status == 1 || status == 2) {
                return Ok(());
            }
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
}

#[cfg(not(any(windows, target_os = "macos")))]
mod imp {
    pub fn set(_on: bool) -> Result<(), String> {
        Err("이 시스템에서는 지원하지 않습니다".into())
    }
}

#[cfg(all(test, target_os = "macos"))]
mod tests {
    /// The SMAppService calls are well-formed (a test binary has no bundle, so the system
    /// answers with an error or, outside an app, a no-op; neither may crash).
    #[test]
    fn mac_calls_return() {
        let _ = super::set(false);
    }
}
