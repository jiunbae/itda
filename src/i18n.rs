//! Interface language: Korean or English.
//!
//! The web UI translates itself (web/i18n.js with the catalogs in web/locales/); this module
//! picks the language the app's own windows open in, holds the catalog files they load, and
//! translates the text Rust renders itself (menus, dialogs, the Windows viewer, errors):
//! `tr!("menu.quit")`, `tr!("err.port", port = p, error = e)`, with the messages in
//! src/locales/{ko,en}.rs keyed by stable IDs. Korean is the source language.
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
    *F.get_or_init(|| match std::env::var("ITDA_LANG").ok().as_deref() {
        Some("ko") => Some(KO),
        Some("en") => Some(EN),
        // Unit tests check the Korean text unless they ask otherwise.
        _ if cfg!(test) => Some(KO),
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
/// ITDA_LANG also wins over Settings → Language in the page, as it does here.
pub fn init_script() -> String {
    let lock = if std::env::var_os("ITDA_LANG").is_some() && forced().is_some() { "window.ITDA_LANG_FORCED=true;" } else { "" };
    format!("window.ITDA_LANG={:?};window.ITDA_OS_LANG={:?};{lock}", lang(), forced().map_or(os_lang(), name))
}

#[path = "locales/ko.rs"]
mod ko;
#[path = "locales/en.rs"]
mod en;
type Table = &'static [(&'static str, &'static str)];
const KO_ALL: Table = ko::MESSAGES;
const EN_ALL: Table = en::MESSAGES;

fn find(table: Table, key: &str) -> Option<&'static str> {
    table.iter().find(|(k, _)| *k == key).map(|(_, v)| *v)
}

/// A message in the current language (Korean when English lacks it; the ID when neither has it).
pub fn text(key: &'static str) -> &'static str {
    text_in(lang(), key)
}

pub fn text_in(l: &str, key: &'static str) -> &'static str {
    let t = if l == "ko" { KO_ALL } else { EN_ALL };
    find(t, key).or_else(|| find(KO_ALL, key)).unwrap_or(key)
}

/// `{name}` placeholders filled in; unknown ones stay as they are.
pub fn fill(t: &str, vars: &[(&str, &dyn std::fmt::Display)]) -> String {
    let mut out = String::with_capacity(t.len() + 16);
    let mut rest = t;
    while let Some(i) = rest.find('{') {
        out.push_str(&rest[..i]);
        let after = &rest[i + 1..];
        let hit = after.find('}').and_then(|j| vars.iter().find(|(n, _)| *n == &after[..j]).map(|(_, v)| (j, v)));
        match hit {
            Some((j, v)) => {
                out.push_str(&v.to_string());
                rest = &after[j + 1..];
            }
            None => {
                out.push('{');
                rest = after;
            }
        }
    }
    out.push_str(rest);
    out
}

/// `tr!("id")` → `&'static str`; `tr!("id", name = value, ...)` → `String` with `{name}` filled.
macro_rules! tr {
    ($key:literal) => {
        $crate::i18n::text($key)
    };
    ($key:literal, $($name:ident = $value:expr),+ $(,)?) => {
        $crate::i18n::fill($crate::i18n::text($key), &[$((stringify!($name), &$value as &dyn ::std::fmt::Display)),+])
    };
}

#[cfg(test)]
mod tests {
    use super::*;

    fn holes(s: &str) -> Vec<String> {
        let mut v: Vec<String> = s.split('{').skip(1).filter_map(|p| p.split_once('}').map(|(n, _)| n.to_string())).filter(|n| !n.is_empty() && n.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')).collect();
        v.sort();
        v.dedup();
        v
    }

    #[test]
    fn rust_catalogs_match() {
        let mut seen = std::collections::HashSet::new();
        for (k, _) in KO_ALL.iter() {
            assert!(seen.insert(*k), "{k}: defined twice in Korean");
        }
        let mut seen_en = std::collections::HashSet::new();
        for (k, v) in EN_ALL.iter() {
            assert!(seen_en.insert(*k), "{k}: defined twice in English");
            let ko = find(KO_ALL, k).unwrap_or_else(|| panic!("{k}: in English but not Korean"));
            assert_eq!(holes(ko), holes(v), "{k}: placeholders differ");
            // Language names are written in their own language (lang.ko is 한국어 in both).
            assert!(k.starts_with("lang.") || !v.chars().any(|c| ('가'..='힣').contains(&c)), "{k}: English text has Hangul");
        }
        for (k, _) in KO_ALL.iter() {
            assert!(find(EN_ALL, k).is_some(), "{k}: in Korean but not English");
        }
    }

    #[test]
    fn every_tr_id_exists() {
        // tr!("id", ...) in the sources names a message in the catalogs.
        let src = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        let mut stack = vec![src];
        while let Some(d) = stack.pop() {
            for e in std::fs::read_dir(&d).unwrap().flatten() {
                let p = e.path();
                if p.is_dir() { stack.push(p); continue; }
                if p.extension().is_none_or(|x| x != "rs") || p.ends_with("i18n.rs") { continue; }
                let s = std::fs::read_to_string(&p).unwrap();
                for (i, _) in s.match_indices("tr!(\"") {
                    if s[..i].chars().next_back().is_some_and(|c| c.is_alphanumeric() || c == '_') { continue; }
                    let id = s[i + 5..].split('"').next().unwrap();
                    assert!(find(KO_ALL, id).is_some(), "{}: tr!(\"{id}\") is not in src/locales", p.display());
                }
            }
        }
    }

    #[test]
    fn filling_placeholders() {
        assert_eq!(fill("포트 {port}을(를) 열 수 없습니다: {error}", &[("port", &5900), ("error", &"busy")]), "포트 5900을(를) 열 수 없습니다: busy");
        assert_eq!(fill("{a} {missing} {", &[("a", &1)]), "1 {missing} {");
        assert_eq!(text_in("en", "lang.en"), "English");
        assert_eq!(text_in("ko", "nope.nothing"), "nope.nothing");
    }

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
