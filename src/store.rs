//! Persistent data in %APPDATA%\Itda: the address book (hosts.json), app settings
//! (settings.json) and thumbnails. Passwords live in Windows Credential Manager.
//! Only the home process writes these files.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

pub fn dir() -> PathBuf {
    static DIR: std::sync::OnceLock<PathBuf> = std::sync::OnceLock::new();
    DIR.get_or_init(|| {
        let base = std::env::var_os("APPDATA").map(PathBuf::from).unwrap_or_else(std::env::temp_dir);
        let d = base.join(crate::APP);
        // The app was called mvnc before: carry its data over once.
        let old = base.join("mvnc");
        if !d.exists() && old.is_dir() && std::fs::rename(&old, &d).is_ok() {
            crate::log!("moved {} -> {}", old.display(), d.display());
        }
        let _ = std::fs::create_dir_all(&d);
        d
    })
    .clone()
}

pub fn thumb_path(id: &str) -> PathBuf {
    let d = dir().join("thumbs");
    let _ = std::fs::create_dir_all(&d);
    d.join(format!("{id}.png"))
}

/// Per-connection viewer options.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq)]
#[serde(default)]
pub struct HostOpts {
    /// 1:1 pixels with scrollbars instead of fit-to-window.
    pub actual: bool,
    pub fullscreen: bool,
    pub view_only: bool,
    /// Ctrl ↔ Cmd.
    pub swap: bool,
    /// Korean/English key sends Caps Lock (else Ctrl+Space).
    pub hangul_caps: bool,
    pub clipboard: bool,
    pub reconnect: bool,
    pub stats: bool,
}

impl Default for HostOpts {
    fn default() -> Self {
        HostOpts {
            actual: false,
            fullscreen: false,
            view_only: false,
            swap: false,
            hangul_caps: false,
            clipboard: true,
            reconnect: true,
            stats: false,
        }
    }
}

/// SSH tunnel settings (the Mac's Remote Login).
#[derive(Serialize, Deserialize, Clone, Default, Debug, PartialEq)]
#[serde(default)]
pub struct Ssh {
    pub on: bool,
    /// Empty = the Mac account name.
    pub user: String,
    /// 0 = 22.
    pub port: u16,
}

#[derive(Serialize, Deserialize, Clone, Default, Debug)]
#[serde(default)]
pub struct Host {
    pub id: String,
    /// Display name; empty = use the address.
    pub name: String,
    /// Address, optionally with :port.
    pub host: String,
    pub user: String,
    pub group: String,
    pub favorite: bool,
    /// Unix seconds of the last successful connection (0 = never).
    pub last_used: u64,
    /// Server-reported desktop name and size from the last connection.
    pub remote_name: String,
    pub remote_size: String,
    pub opts: HostOpts,
    pub ssh: Ssh,
    /// Shared account to sign in with (empty = this connection's own name/password).
    pub account: String,
    /// "" = VNC (Mac Screen Sharing, Itda sharing, any VNC server); "rdp" = Windows Remote Desktop.
    pub kind: String,
    /// What the device is, for wording and defaults: "mac" | "windows" | "linux" ("" = Mac, from
    /// before this was stored).
    pub os: String,
}

impl Host {
    pub fn is_rdp(&self) -> bool {
        self.kind == "rdp"
    }
}

/// Credentials shared by several connections (one Mac account on many Macs).
#[derive(Serialize, Deserialize, Clone, Default, Debug)]
#[serde(default)]
pub struct Account {
    pub id: String,
    pub name: String,
    /// Mac account short name.
    pub user: String,
    /// Private key file for SSH (empty = ssh's defaults and agent).
    pub ssh_key: String,
}

/// What a connection actually signs in with.
#[derive(Clone, Default, Debug)]
pub struct Login {
    pub user: String,
    pub pass: Option<String>,
    pub ssh_key: String,
    pub key_pass: Option<String>,
}

pub fn resolve(h: &Host, accounts: &[Account]) -> Login {
    match accounts.iter().find(|a| !h.account.is_empty() && a.id == h.account) {
        Some(a) => Login {
            user: a.user.clone(),
            pass: get_password(&acct_key(&a.id)),
            ssh_key: a.ssh_key.clone(),
            key_pass: get_password(&acct_keypass(&a.id)),
        },
        None => Login { user: h.user.clone(), pass: get_password(&h.id), ..Login::default() },
    }
}

/// Credential Manager keys for an account's secrets.
pub fn acct_key(id: &str) -> String {
    format!("acct-{id}")
}
pub fn acct_keypass(id: &str) -> String {
    format!("acctkey-{id}")
}

pub fn load_accounts() -> Vec<Account> {
    std::fs::read(dir().join("accounts.json")).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default()
}

pub fn save_accounts(a: &[Account]) {
    write_atomic("accounts.json", &serde_json::to_vec_pretty(a).unwrap_or_default());
}

impl Host {
    pub fn label(&self) -> &str {
        if self.name.is_empty() { &self.host } else { &self.name }
    }
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(default)]
pub struct Settings {
    /// "system" | "light" | "dark"
    pub theme: String,
    /// Closing the home window keeps Itda in the tray.
    pub tray: bool,
    pub thumbnails: bool,
    pub notify: bool,
    /// Viewer hotkey modifiers: "ctrl_alt" | "ctrl_shift"
    pub hotkey: String,
    /// In-session toolbar: "auto" (slides in at the top edge) | "pinned" | "hidden"
    pub toolbar: String,
    /// Home layout: "grid" | "list"
    pub view: String,
    /// Shown the "still running in the tray" notice once.
    pub tray_hint_shown: bool,
    /// Home window zoom in percent (Ctrl +/-, Ctrl+wheel).
    pub zoom: u16,
    /// The first-run guide was finished or skipped.
    pub onboarded: bool,
    /// Options for new connections.
    pub defaults: HostOpts,
    /// Web viewer: other devices open the address book in a browser.
    pub web: Web,
    /// Share this PC: a VNC server for other devices.
    pub share: Share,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(default)]
pub struct Share {
    pub on: bool,
    pub port: u16,
    /// Ask the person at this PC before each viewer gets in.
    pub ask: bool,
    /// A Mac's ⌘ acts as Ctrl on this PC.
    pub cmd_as_ctrl: bool,
    pub view_only: bool,
}

impl Default for Share {
    fn default() -> Self {
        Share { on: false, port: 5900, ask: true, cmd_as_ctrl: true, view_only: false }
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(default)]
pub struct Web {
    pub on: bool,
    pub port: u16,
}

impl Default for Web {
    fn default() -> Self {
        Web { on: false, port: 8780 }
    }
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            theme: "system".into(),
            tray: true,
            thumbnails: true,
            notify: true,
            hotkey: "ctrl_alt".into(),
            toolbar: "auto".into(),
            view: "grid".into(),
            tray_hint_shown: false,
            zoom: 100,
            onboarded: false,
            defaults: HostOpts::default(),
            web: Web::default(),
            share: Share::default(),
        }
    }
}

pub fn now() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs())
}

pub fn new_id() -> String {
    let mut b = [0u8; 6];
    let _ = getrandom::getrandom(&mut b);
    b.iter().map(|x| format!("{x:02x}")).collect()
}

fn write_atomic(name: &str, data: &[u8]) {
    let d = dir();
    let tmp = d.join(format!("{name}.tmp"));
    if std::fs::write(&tmp, data).is_ok() {
        let _ = std::fs::rename(&tmp, d.join(name));
    }
}

pub fn load_settings() -> Settings {
    std::fs::read(dir().join("settings.json"))
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_else(migrate_settings)
}

pub fn save_settings(s: &Settings) {
    write_atomic("settings.json", &serde_json::to_vec_pretty(s).unwrap_or_default());
}

pub fn load_hosts() -> Vec<Host> {
    match std::fs::read(dir().join("hosts.json")) {
        Ok(b) => serde_json::from_slice(&b).unwrap_or_else(|e| {
            // Never let the next save silently replace an address book we failed to read.
            let bad = dir().join(format!("hosts.json.bad-{}", now()));
            crate::log!("hosts.json unreadable ({e}); kept as {}", bad.display());
            let _ = std::fs::rename(dir().join("hosts.json"), bad);
            Vec::new()
        }),
        Err(_) => {
            let hosts = migrate_hosts();
            if !hosts.is_empty() {
                save_hosts(&hosts);
            }
            hosts
        }
    }
}

pub fn save_hosts(hosts: &[Host]) {
    write_atomic("hosts.json", &serde_json::to_vec_pretty(hosts).unwrap_or_default());
}

// ---------------------------------------------------------------- migration from 0.1 files

/// settings.ini (key=value) → defaults for new connections.
fn old_opts() -> HostOpts {
    let mut o = HostOpts::default();
    let txt = std::fs::read_to_string(dir().join("settings.ini")).unwrap_or_default();
    for line in txt.lines() {
        let Some((k, v)) = line.split_once('=') else { continue };
        let on = v.trim() == "1";
        match k.trim() {
            "actual" => o.actual = on,
            "stats" => o.stats = on,
            "hangul_caps" => o.hangul_caps = on,
            "no_clip" => o.clipboard = !on,
            _ => {}
        }
    }
    o
}

fn migrate_settings() -> Settings {
    Settings { defaults: old_opts(), ..Settings::default() }
}

/// hosts.txt (host\tuser\tswap) or last.txt → hosts, moving saved passwords to id-based entries.
fn migrate_hosts() -> Vec<Host> {
    let d = dir();
    let mut rows: Vec<(String, String, bool)> = Vec::new();
    if let Ok(txt) = std::fs::read_to_string(d.join("hosts.txt")) {
        for l in txt.lines() {
            let mut f = l.split('\t');
            if let Some(h) = f.next().filter(|h| !h.is_empty()) {
                rows.push((h.into(), f.next().unwrap_or("").into(), f.next() == Some("1")));
            }
        }
    } else if let Ok(txt) = std::fs::read_to_string(d.join("last.txt")) {
        let mut l = txt.lines();
        if let Some(h) = l.next().filter(|h| !h.is_empty()) {
            rows.push((h.into(), l.next().unwrap_or("").into(), l.next() == Some("1")));
        }
    }
    let base = old_opts();
    let t = now();
    rows.into_iter()
        .enumerate()
        .map(|(i, (host, user, swap))| {
            let h = Host {
                id: new_id(),
                host: host.clone(),
                user: user.clone(),
                last_used: t.saturating_sub(i as u64),
                opts: HostOpts { swap, ..base },
                ..Host::default()
            };
            let old = format!("mvnc:{user}@{host}");
            if let Some(p) = cred_read(&old) {
                set_password(&h.id, Some(&p));
                cred_delete(&old);
            }
            crate::log!("migrated host {user}@{host} -> {}", h.id);
            h
        })
        .collect()
}

// ---------------------------------------------------------------- Credential Manager

use windows_sys::Win32::Security::Credentials::*;

fn w(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(Some(0)).collect()
}

fn target(id: &str) -> String {
    format!("Itda/{id}")
}

fn cred_read(target: &str) -> Option<String> {
    unsafe {
        let mut p: *mut CREDENTIALW = std::ptr::null_mut();
        if CredReadW(w(target).as_ptr(), CRED_TYPE_GENERIC, 0, &mut p) == 0 {
            return None;
        }
        let c = &*p;
        let blob = std::slice::from_raw_parts(c.CredentialBlob as *const u16, c.CredentialBlobSize as usize / 2);
        let s = String::from_utf16_lossy(blob);
        CredFree(p as _);
        Some(s)
    }
}

fn cred_delete(target: &str) {
    unsafe { CredDeleteW(w(target).as_ptr(), CRED_TYPE_GENERIC, 0) };
}

/// Hand Remote Desktop its sign-in (the same TERMSRV/<host> entry mstsc's "remember me" makes).
pub fn set_rdp_cred(host: &str, user: &str, pass: &str) {
    let target = w(&format!("TERMSRV/{host}"));
    let mut blob: Vec<u16> = pass.encode_utf16().collect();
    let mut u = w(user);
    unsafe {
        let mut cred: CREDENTIALW = std::mem::zeroed();
        cred.Type = CRED_TYPE_DOMAIN_PASSWORD;
        cred.TargetName = target.as_ptr() as *mut u16;
        cred.CredentialBlobSize = (blob.len() * 2) as u32;
        cred.CredentialBlob = blob.as_mut_ptr() as *mut u8;
        cred.Persist = CRED_PERSIST_LOCAL_MACHINE;
        cred.UserName = u.as_mut_ptr();
        CredWriteW(&cred, 0);
    }
}

/// Password viewers use to see this PC (VNC uses at most 8 characters), created on first use.
pub fn share_password(set: Option<&str>, reset: bool) -> String {
    let key = if std::env::var_os("ITDA_TEST_HTTP").is_some() { "share-pw-test" } else { "share-pw" };
    if let Some(p) = set {
        let p: String = p.chars().take(8).collect();
        set_password(key, Some(&p));
        return p;
    }
    if !reset {
        if let Some(p) = cred_read(&target(key)) {
            return p;
        }
    }
    // No look-alikes (0/O, 1/l/I) so it can be read out and typed on a phone.
    const A: &[u8] = b"abcdefghjkmnpqrstuvwxyz23456789";
    let mut b = [0u8; 8];
    let _ = getrandom::getrandom(&mut b);
    let p: String = b.iter().map(|x| A[*x as usize % A.len()] as char).collect();
    set_password(key, Some(&p));
    p
}

/// The web viewer's access PIN (6 digits), created on first use.
pub fn web_pin(reset: bool) -> String {
    // Test runs keep their own PIN so they never touch the real one.
    let key = if std::env::var_os("ITDA_TEST_HTTP").is_some() { "web-pin-test" } else { "web-pin" };
    let t = target(key);
    if !reset {
        if let Some(p) = cred_read(&t) {
            return p;
        }
    }
    let mut b = [0u8; 4];
    let _ = getrandom::getrandom(&mut b);
    let pin = format!("{:06}", u32::from_le_bytes(b) % 1_000_000);
    set_password(key, Some(&pin));
    pin
}

pub fn get_password(id: &str) -> Option<String> {
    cred_read(&target(id)).or_else(|| {
        // Saved under the old app name: move it.
        let old = format!("mvnc/{id}");
        let p = cred_read(&old)?;
        set_password(id, Some(&p));
        cred_delete(&old);
        Some(p)
    })
}

/// Store (Some) or delete (None) the saved password of host `id`.
pub fn set_password(id: &str, pass: Option<&str>) {
    let t = target(id);
    let Some(pass) = pass else { return cred_delete(&t) };
    let target = w(&t);
    let mut blob: Vec<u16> = pass.encode_utf16().collect();
    let mut user = w(crate::APP);
    unsafe {
        let mut cred: CREDENTIALW = std::mem::zeroed();
        cred.Type = CRED_TYPE_GENERIC;
        cred.TargetName = target.as_ptr() as *mut u16;
        cred.CredentialBlobSize = (blob.len() * 2) as u32;
        cred.CredentialBlob = blob.as_mut_ptr() as *mut u8;
        cred.Persist = CRED_PERSIST_LOCAL_MACHINE;
        cred.UserName = user.as_mut_ptr();
        CredWriteW(&cred, 0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn credential_roundtrip() {
        let id = "test-0000";
        set_password(id, Some("p@ss 한글 ✓"));
        assert_eq!(get_password(id).as_deref(), Some("p@ss 한글 ✓"));
        set_password(id, None);
        assert_eq!(get_password(id), None);
    }

    #[test]
    fn legacy_credential_moves() {
        let id = "test-0001";
        let old = format!("mvnc/{id}");
        set_password(id, None);
        // Write an entry under the pre-rename target, as 0.2 did.
        unsafe {
            let t = w(&old);
            let mut blob: Vec<u16> = "old pw".encode_utf16().collect();
            let mut cred: CREDENTIALW = std::mem::zeroed();
            cred.Type = CRED_TYPE_GENERIC;
            cred.TargetName = t.as_ptr() as *mut u16;
            cred.CredentialBlobSize = (blob.len() * 2) as u32;
            cred.CredentialBlob = blob.as_mut_ptr() as *mut u8;
            cred.Persist = CRED_PERSIST_LOCAL_MACHINE;
            CredWriteW(&cred, 0);
        }
        assert_eq!(get_password(id).as_deref(), Some("old pw"));
        assert_eq!(cred_read(&old), None, "old entry removed");
        assert_eq!(cred_read(&target(id)).as_deref(), Some("old pw"));
        set_password(id, None);
    }

    #[test]
    fn host_json_defaults() {
        let h: Host = serde_json::from_str(r#"{"id":"a","host":"mac.local"}"#).unwrap();
        assert!(h.opts.clipboard && h.opts.reconnect && !h.opts.swap);
        assert_eq!(h.label(), "mac.local");
    }
}
