//! Interface language: Korean or English.
//!
//! The web UI translates itself (web/i18n.js with the catalogs in web/locales/); this module
//! picks the language the app's own windows open in and holds the catalog files they load.
//! Order: the `ITDA_LANG` environment variable (`ko`/`en`, for tests and recordings), then
//! Settings → Language (`auto`/`ko`/`en`), then the system's language: Korean when it
//! starts with "ko", otherwise English.

use std::sync::atomic::{AtomicU8, Ordering::Relaxed};
use std::sync::OnceLock;

pub const WEB_I18N: &str = include_str!("../web/i18n.js");
pub const WEB_KO: &str = include_str!("../web/locales/ko.js");
pub const WEB_EN: &str = include_str!("../web/locales/en.js");

const UNSET: u8 = 0;
const KO: u8 = 1;
const EN: u8 = 2;
static CHOSEN: AtomicU8 = AtomicU8::new(UNSET);

fn code(tag: &str) -> u8 {
    if tag.trim().to_ascii_lowercase().starts_with("ko") { KO } else { EN }
}

fn name(c: u8) -> &'static str {
    if c == KO { "ko" } else { "en" }
}

fn forced() -> Option<u8> {
    static F: OnceLock<Option<u8>> = OnceLock::new();
    *F.get_or_init(|| match std::env::var("ITDA_LANG").ok()?.as_str() {
        "ko" => Some(KO),
        "en" => Some(EN),
        _ => None,
    })
}

/// The system's language, "ko" or "en".
pub fn os_lang() -> &'static str {
    static OS: OnceLock<u8> = OnceLock::new();
    name(*OS.get_or_init(|| system_tag().map_or(EN, |t| code(&t))))
}

#[cfg(target_os = "macos")]
fn system_tag() -> Option<String> {
    // The first of System Settings → Language & Region → Preferred Languages.
    let langs = objc2_foundation::NSLocale::preferredLanguages();
    langs.iter().next().map(|s| s.to_string())
}

#[cfg(windows)]
fn system_tag() -> Option<String> {
    // The display language: Korean is primary language 0x12.
    let id = unsafe { windows_sys::Win32::Globalization::GetUserDefaultUILanguage() };
    Some(if id & 0x3ff == 0x12 { "ko".into() } else { "en".into() })
}

#[cfg(not(any(target_os = "macos", windows)))]
fn system_tag() -> Option<String> {
    ["LC_ALL", "LC_MESSAGES", "LANG"].iter().find_map(|k| std::env::var(k).ok().filter(|v| !v.is_empty()))
}

/// What a Settings → Language value shows: "ko" or "en".
pub fn resolve(pref: &str) -> &'static str {
    if let Some(f) = forced() {
        return name(f);
    }
    match pref {
        "ko" => "ko",
        "en" => "en",
        _ => os_lang(),
    }
}

/// Settings → Language changed (or was loaded).
pub fn set_pref(pref: &str) {
    CHOSEN.store(code(resolve(pref)), Relaxed);
}

/// The language the app shows now, "ko" or "en".
pub fn lang() -> &'static str {
    match CHOSEN.load(Relaxed) {
        UNSET => {
            let l = resolve(&crate::store::load_settings().lang);
            CHOSEN.store(code(l), Relaxed);
            l
        }
        c => name(c),
    }
}

/// Script for a web view the app opens: the page starts in the app's language and resolves
/// "auto" with the system's (see web/i18n.js).
pub fn init_script() -> String {
    format!("window.ITDA_LANG={:?};window.ITDA_OS_LANG={:?};", lang(), forced().map_or(os_lang(), name))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn language_tags() {
        assert_eq!(name(code("ko-KR")), "ko");
        assert_eq!(name(code("ko")), "ko");
        assert_eq!(name(code("KO_kr.UTF-8")), "ko");
        assert_eq!(name(code("en-US")), "en");
        assert_eq!(name(code("ja-JP")), "en");
        assert_eq!(name(code("")), "en");
    }

    #[test]
    fn explicit_choice_wins_over_the_system() {
        if forced().is_none() {
            assert_eq!(resolve("ko"), "ko");
            assert_eq!(resolve("en"), "en");
            assert_eq!(resolve("auto"), os_lang());
            assert_eq!(resolve("bogus"), os_lang());
        }
    }

    #[test]
    fn catalogs_are_embedded() {
        assert!(WEB_I18N.contains("window.I18N"));
        assert!(WEB_KO.starts_with("//") && WEB_KO.contains("I18N.add('ko'"));
        assert!(WEB_EN.contains("I18N.add('en'"));
    }
}
