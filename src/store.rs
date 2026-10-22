//! Persistent data in %APPDATA%\Itda (macOS: ~/Library/Application Support/Itda): the
//! address book (hosts.json), app settings (settings.json) and thumbnails. Passwords live in
//! Windows Credential Manager (macOS: the login Keychain). Only the home process (on macOS,
//! the gateway) writes these files.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

pub fn dir() -> PathBuf {
    static DIR: std::sync::OnceLock<PathBuf> = std::sync::OnceLock::new();
    DIR.get_or_init(|| {
        #[cfg(windows)]
        let base = std::env::var_os("APPDATA").map(PathBuf::from).unwrap_or_else(std::env::temp_dir);
        #[cfg(not(windows))]
        let base = std::env::var_os("ITDA_DATA_DIR")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join("Library/Application Support")))
            .unwrap_or_else(std::env::temp_dir);
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

/// The connection's last screen, saved by its session.
pub fn thumb_path(id: &str) -> PathBuf {
    let d = dir().join("thumbs");
    let _ = std::fs::create_dir_all(&d);
    d.join(format!("{id}.png"))
}

/// A picture the user chose for the card, to tell devices apart.
pub fn custom_thumb_path(id: &str) -> PathBuf {
    thumb_path(id).with_extension("custom.png")
}

/// What the card shows: the chosen picture, else the last screen.
pub fn shown_thumb_path(id: &str) -> PathBuf {
    Some(custom_thumb_path(id)).filter(|p| p.is_file()).unwrap_or_else(|| thumb_path(id))
}

pub fn remove_thumbs(id: &str) {
    let _ = std::fs::remove_file(thumb_path(id));
    let _ = std::fs::remove_file(custom_thumb_path(id));
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
    /// Remote Desktop (and VNC servers that allow it) follow the window's size.
    pub resize: bool,
    /// How the remote screen is divided for one-display views and per-display windows:
    /// 0 = the displays the server reports (one desktop if it reports none), or a manual
    /// split of the framebuffer into screen regions — 1 = two side by side, 2 = two stacked,
    /// 3 = three side by side, 4 = 2×2. Manual regions are not detected monitors.
    pub monitor_layout: u8,
    /// What a multi-display remote opens as: 0 = one display (monitor_id, else the first),
    /// 1 = all of them in one window, 2 = one window per display.
    pub monitor_mode: u8,
    /// The display picked for mode 0: the server's screen id, or for a manual split the
    /// region's index (0-based). None = the first.
    pub monitor_id: Option<u32>,
    /// Per-display windows (mode 2) open full screen, each on its monitor.
    pub monitor_fullscreen: bool,
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
            resize: true,
            monitor_layout: 0,
            monitor_mode: 0,
            monitor_id: None,
            monitor_fullscreen: false,
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
    /// The groups joined with ", " (what versions before `groups` read and wrote).
    pub group: String,
    /// Groups this connection is listed under (any number).
    pub groups: Vec<String>,
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
    /// Files use a separate login/connection from the remote screen.
    pub files: crate::files::Options,
    /// Itda's high-performance sharing set up on this Mac (reached over SSH).
    pub hp: Hp,
    /// Its address on the local network (no port), tried when `host` doesn't answer, e.g. a
    /// Tailscale name while Tailscale is down there. Learned from Tailscale's direct path.
    pub lan: String,
    /// Set while connecting at `lan`: the device's own name, whose identity it must show.
    #[serde(skip)]
    pub key_alias: String,
}

#[derive(Serialize, Deserialize, Clone, Default, Debug, PartialEq)]
#[serde(default)]
pub struct Hp {
    pub on: bool,
    /// Its port on the Mac (127.0.0.1 there); 0 = 5910.
    pub port: u16,
    /// Sharper: the screen at its native (Retina) pixels rather than in points; slower to encode.
    pub native: bool,
}

impl Host {
    /// Groups from `groups`, else from the old comma-separated `group`: trimmed, without
    /// empties or repeats; `group` follows them.
    pub fn normalize_groups(&mut self) {
        let from = if self.groups.is_empty() { self.group.split(',').map(str::to_string).collect() } else { std::mem::take(&mut self.groups) };
        let mut seen = std::collections::HashSet::new();
        self.groups = from.into_iter().map(|g| g.trim().chars().take(40).collect::<String>()).filter(|g| !g.is_empty() && seen.insert(g.to_lowercase())).collect();
        self.group = self.groups.join(", ");
    }

    pub fn is_rdp(&self) -> bool {
        self.kind == "rdp"
    }

    /// Where a connection to `address` (this host's, or its LAN one) is first answered: Remote
    /// Desktop, SSH when the screen goes through it, else the VNC port.
    pub fn probe_addr(&self, address: &str) -> String {
        let t = address.trim();
        if self.is_rdp() {
            if t.rsplit_once(':').is_some_and(|(_, p)| p.parse::<u16>().is_ok()) { t.to_string() } else { format!("{t}:3389") }
        } else if self.ssh.on || self.hp.on {
            let (host, _) = crate::tunnel::split_host(t, 5900);
            let port = if self.ssh.port == 0 { 22 } else { self.ssh.port };
            if host.contains(':') { format!("[{host}]:{port}") } else { format!("{host}:{port}") }
        } else {
            crate::rfb::addr(t)
        }
    }

    /// This host at its LAN address (keeping any port given with `host`), bound to the
    /// identity it has at its own address (`key_alias`).
    pub fn at_lan(&self) -> Host {
        let (name, port) = crate::tunnel::split_host(&self.host, 0);
        let lan = self.lan.trim();
        let mut h = self.clone();
        h.host = if port == 0 { lan.to_string() } else if lan.contains(':') { format!("[{lan}]:{port}") } else { format!("{lan}:{port}") };
        h.key_alias = name;
        h
    }

    /// Can a screen connection go to the LAN address and still check it is this device?
    /// Over SSH (its trusted host key, also for HP) or, on macOS, Remote Desktop with a server
    /// key seen at its own address. Never plain VNC: nothing there proves who answers.
    pub fn lan_fallback(&self) -> bool {
        cfg!(target_os = "macos") && !self.lan.trim().is_empty() && (self.ssh.on || self.hp.on || (self.is_rdp() && rdp_key(&self.id).is_some()))
    }

    /// The host to connect to: itself, or at its LAN address when only that answers (and
    /// the device there can be checked, see lan_fallback). Waits up to ANSWER for each.
    pub fn reachable(&self) -> Host {
        if !self.lan_fallback() || answers(&self.probe_addr(&self.host)) {
            return self.clone();
        }
        let alt = self.at_lan();
        if answers(&alt.probe_addr(&alt.host)) {
            crate::log!("{}: not answering, trying its LAN address {}", self.host, alt.host);
            return alt;
        }
        self.clone()
    }
}

const ANSWER: std::time::Duration = std::time::Duration::from_millis(1500);

/// Does something accept a TCP connection at `addr` ("host:port") soon?
pub fn answers(addr: &str) -> bool {
    use std::net::ToSocketAddrs;
    addr.to_socket_addrs().is_ok_and(|mut it| it.any(|sa| std::net::TcpStream::connect_timeout(&sa, ANSWER).is_ok()))
}

/// The Remote Desktop server key (SHA-256, hex) each saved host had when last reached at its
/// own address: the LAN address must show the same one before a login goes there.
fn rdp_keys() -> std::collections::HashMap<String, String> {
    std::fs::read(dir().join("rdp-keys.json")).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default()
}

pub fn rdp_key(id: &str) -> Option<String> {
    rdp_keys().remove(id)
}

/// Forget a connection's Remote Desktop key (its next connection learns it again).
pub fn forget_rdp_key(id: &str) {
    static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
    let _g = LOCK.lock().unwrap();
    let mut keys = rdp_keys();
    if keys.remove(id).is_some() {
        write_atomic("rdp-keys.json", &serde_json::to_vec(&keys).unwrap_or_default());
    }
}

pub fn set_rdp_key(id: &str, key: &str) {
    static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
    let _g = LOCK.lock().unwrap();
    let mut keys = rdp_keys();
    if keys.get(id).map(String::as_str) != Some(key) {
        keys.insert(id.to_string(), key.to_string());
        write_atomic("rdp-keys.json", &serde_json::to_vec(&keys).unwrap_or_default());
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
    /// How far a wheel notch scrolls a Mac: "normal" (Windows' lines per notch) | "fast" (×2) | "faster" (×3)
    pub mac_scroll: String,
    /// Rolling the wheel quickly scrolls a Mac further, like a Mac's own mouse.
    pub scroll_accel: bool,
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
    /// Tell other Itdas on the network that Itda runs here (Bonjour `_itda._tcp`).
    pub announce: bool,
    /// Look for a newer release at launch and once a day (installing still asks).
    pub auto_update: bool,
    /// Start Itda (in the notification area on Windows) when the person here logs in.
    pub login_start: bool,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(default)]
pub struct Share {
    pub on: bool,
    pub port: u16,
    /// Ask the person at this PC before each viewer gets in.
    pub ask: bool,
    /// ...except devices on this network or this tailnet: the password alone lets them in.
    pub trust_near: bool,
    /// A Mac's ⌘ acts as Ctrl on this PC.
    pub cmd_as_ctrl: bool,
    pub view_only: bool,
}

impl Default for Share {
    fn default() -> Self {
        // Nearby trust is off for new installs: an address range doesn't prove a device is
        // yours (shared Wi-Fi). Settings saved before keep their own choice.
        Share { on: false, port: 5900, ask: true, trust_near: false, cmd_as_ctrl: true, view_only: false }
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
            mac_scroll: "fast".into(),
            scroll_accel: true,
            view: "grid".into(),
            tray_hint_shown: false,
            zoom: 100,
            onboarded: false,
            defaults: HostOpts::default(),
            web: Web::default(),
            share: Share::default(),
            announce: true,
            auto_update: true,
            login_start: false,
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
        Ok(b) => serde_json::from_slice::<Vec<Host>>(&b).map(|mut hs| {
            hs.iter_mut().for_each(Host::normalize_groups);
            hs
        }).unwrap_or_else(|e| {
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

// ---------------------------------------------------------------- secrets

fn target(id: &str) -> String {
    format!("Itda/{id}")
}

#[cfg(windows)]
use windows_sys::Win32::Security::Credentials::*;

#[cfg(windows)]
fn w(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(Some(0)).collect()
}

#[cfg(windows)]
fn cred_read_checked(target: &str) -> std::io::Result<Option<String>> {
    unsafe {
        let mut p: *mut CREDENTIALW = std::ptr::null_mut();
        if CredReadW(w(target).as_ptr(), CRED_TYPE_GENERIC, 0, &mut p) == 0 {
            let code=windows_sys::Win32::Foundation::GetLastError();
            if code==windows_sys::Win32::Foundation::ERROR_NOT_FOUND {return Ok(None);}
            return Err(std::io::Error::other("Windows 자격 증명 관리자에서 기존 로그인 정보를 읽지 못했습니다"));
        }
        let c = &*p;
        let result=if c.CredentialBlobSize==0 {Ok(String::new())} else {
            let blob=std::slice::from_raw_parts(c.CredentialBlob as *const u16,c.CredentialBlobSize as usize/2);
            String::from_utf16(blob).map_err(|_|std::io::Error::other("저장된 로그인 정보의 문자를 읽지 못했습니다"))
        };
        CredFree(p as _);
        result.map(Some)
    }
}

#[cfg(windows)]
fn cred_delete(target: &str) {
    unsafe { CredDeleteW(w(target).as_ptr(), CRED_TYPE_GENERIC, 0) };
}

#[cfg(windows)]
fn cred_write(target: &str, pass: &str) {
    let target = w(target);
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

// macOS: generic passwords in the login Keychain (service "Itda", account = target), through
// the system `security` tool so no extra crates are needed.
#[cfg(not(windows))]
const KEYCHAIN_SERVICE: &str = "Itda";

#[cfg(not(windows))]
fn security(args: &[&str]) -> Option<std::process::Output> {
    std::process::Command::new("/usr/bin/security").args(args).stdin(std::process::Stdio::null()).output().ok()
}

#[cfg(not(windows))]
fn security_password(printed:&str)->std::io::Result<String> {
    let value=printed.lines().find_map(|line|line.strip_prefix("password: ")).ok_or_else(||std::io::Error::other("키체인의 응답을 읽지 못했습니다"))?;
    if let Some(hex)=value.strip_prefix("0x") {
        let hex=hex.split_whitespace().next().unwrap_or("");
        if hex.len()%2!=0 || !hex.is_ascii(){return Err(std::io::Error::other("키체인의 비밀번호 형식이 올바르지 않습니다"));}
        let bytes=zeroize::Zeroizing::new((0..hex.len()).step_by(2).map(|i|u8::from_str_radix(&hex[i..i+2],16)).collect::<Result<Vec<_>,_>>().map_err(|_|std::io::Error::other("키체인의 비밀번호 형식이 올바르지 않습니다"))?);
        return std::str::from_utf8(&bytes).map(str::to_owned).map_err(|_|std::io::Error::other("저장된 로그인 정보의 문자를 읽지 못했습니다"));
    }
    if value.is_empty(){return Ok(String::new());}
    value.strip_prefix('"').and_then(|v|v.strip_suffix('"')).map(str::to_owned).ok_or_else(||std::io::Error::other("키체인의 비밀번호 형식이 올바르지 않습니다"))
}

#[cfg(not(windows))]
fn cred_read_checked(target: &str)->std::io::Result<Option<String>> {
    // -w prints non-ASCII/control bytes as unmarked hex. -g has an explicit
    // 0x prefix, so Korean passwords and literal hex strings remain distinct.
    let mut o=security(&["find-generic-password","-s",KEYCHAIN_SERVICE,"-a",target,"-g"]).ok_or_else(||std::io::Error::other("Mac 키체인을 열지 못했습니다"))?;
    let result=if o.status.success(){security_password(&String::from_utf8_lossy(&o.stderr)).map(Some)}
        // errSecItemNotFound (-25300), returned as an eight-bit process status.
        else if o.status.code()==Some(44){Ok(None)}
        else {Err(std::io::Error::other("Mac 키체인의 기존 로그인 정보를 읽지 못했습니다. 키체인을 잠금 해제한 뒤 다시 시도하세요"))};
    use zeroize::Zeroize;o.stdout.zeroize();o.stderr.zeroize();result
}

fn cred_read(target:&str)->Option<String> {cred_read_checked(target).ok().flatten()}

#[cfg(not(windows))]
fn cred_delete(target: &str) {
    let _ = security(&["delete-generic-password", "-s", KEYCHAIN_SERVICE, "-a", target]);
}

#[cfg(not(windows))]
fn security_arg(value:&str)->String {
    format!("\"{}\"",value.replace('\\',"\\\\").replace('"',"\\\""))
}

#[cfg(not(windows))]
fn cred_write(target: &str, pass: &str) {
    use std::io::Write;
    // The secret goes through stdin (`security -i`), hex-encoded (-X), so it never shows up
    // in the process list. -U updates an existing item.
    let hex=zeroize::Zeroizing::new(pass.bytes().map(|b|format!("{b:02x}")).collect::<String>());
    if target.chars().any(char::is_control) { return; }
    let line=zeroize::Zeroizing::new(format!("add-generic-password -U -s {KEYCHAIN_SERVICE} -a {} -l {} -X \"{}\"\n",security_arg(target),security_arg(&format!("Itda ({target})")),&*hex));
    // Apple's interactive parser has a 4096-byte line buffer. Never let a
    // truncated line replace a valid stored password with partial bytes.
    if line.len()>=4096 {return;}
    let child = std::process::Command::new("/usr/bin/security")
        .arg("-i")
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn();
    if let Ok(mut c) = child {
        if let Some(mut i) = c.stdin.take() {
            let _ = i.write_all(line.as_bytes());
        }
        let _ = c.wait();
    }
}

/// Hand Remote Desktop its sign-in (the same TERMSRV/<host> entry mstsc's "remember me" makes).
#[cfg(windows)]
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
#[cfg(windows)]
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
    match pass {
        Some(p) => cred_write(&t, p),
        None => cred_delete(&t),
    }
}

/// A locked/inaccessible vault must not be treated as an absent rollback value.
pub fn get_password_checked(id:&str)->std::io::Result<Option<String>> {
    match cred_read_checked(&target(id))? {
        Some(value)=>Ok(Some(value)),
        None=>cred_read_checked(&format!("mvnc/{id}")),
    }
}

/// Configuration provisioning must not report success when the OS vault rejected a write.
pub fn set_password_checked(id: &str, pass: Option<&str>) -> std::io::Result<()> {
    if id.chars().any(char::is_control) {return Err(std::io::Error::other("로그인 식별자에 제어 문자가 있습니다"));}
    #[cfg(not(windows))]
    if pass.is_some_and(|p|p.len().saturating_mul(2)+format!("add-generic-password -U -s {KEYCHAIN_SERVICE} -a {} -l {} -X \"\"\n",security_arg(&target(id)),security_arg(&format!("Itda ({})",target(id)))).len()>=4096) {
        return Err(std::io::Error::other("비밀번호가 이 Mac의 보안 저장소 입력 한도를 초과했습니다. 해당 항목을 제외하고 다시 시도하세요"));
    }
    set_password(id,pass);
    let mut got=cred_read_checked(&target(id))?;
    let ok=got.as_deref()==pass;
    if let Some(value)=got.as_mut() {use zeroize::Zeroize;value.zeroize();}
    if ok {
        Ok(())
    } else {
        #[cfg(not(windows))]
        let message="Mac 키체인에 로그인 정보를 저장하지 못했습니다. 로그인 키체인을 잠금 해제한 후 다시 시도하세요";
        #[cfg(windows)]
        let message="Windows 자격 증명 관리자에 로그인 정보를 저장하지 못했습니다. 비밀번호 공유를 해제하면 프로필만 가져올 수 있습니다";
        Err(std::io::Error::other(message))
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn nearby_trust_is_off_for_new_installs_and_kept_for_old_ones() {
        assert!(!super::Settings::default().share.trust_near);
        let old: super::Settings = serde_json::from_str(r#"{"share":{"on":true,"port":5900,"ask":true,"trust_near":true}}"#).unwrap();
        assert!(old.share.trust_near, "a saved choice must survive the new default");
    }

    #[test]
    fn lan_address_keeps_the_port_and_the_way_in() {
        let mut h = super::Host { host: "jiun-home.tail.ts.net".into(), lan: "192.168.32.32".into(), kind: "rdp".into(), ..Default::default() };
        assert_eq!(h.at_lan().host, "192.168.32.32");
        assert_eq!(h.probe_addr(&h.at_lan().host), "192.168.32.32:3389");
        h.host = "jiun-home.tail.ts.net:3390".into();
        assert_eq!(h.at_lan().host, "192.168.32.32:3390");
        assert_eq!(h.probe_addr(&h.at_lan().host), "192.168.32.32:3390");
        h.kind = String::new();
        h.ssh.on = true;
        assert_eq!(h.probe_addr("192.168.32.32:5901"), "192.168.32.32:22");
        // At the LAN address it must prove to be the device named by its own address.
        assert_eq!(h.at_lan().key_alias, "jiun-home.tail.ts.net");
        // Without a LAN address nothing is probed or changed.
        h.lan.clear();
        assert!(!h.lan_fallback());
        assert_eq!(h.reachable().host, h.host);
        // Plain VNC can't check who answers: never sent to the LAN address.
        h.lan = "192.168.32.32".into();
        h.ssh.on = false;
        assert!(!h.lan_fallback());
        // RDP only with a server key seen at its own address.
        h.kind = "rdp".into();
        h.id = "lan-test-no-key".into();
        assert!(!h.lan_fallback());
        h.ssh.on = true;
        h.kind = String::new();
        assert_eq!(h.lan_fallback(), cfg!(target_os = "macos"));
    }

    #[cfg(not(windows))]
    #[test]
    fn keychain_password_format_preserves_unicode_quotes_and_literal_hex() {
        assert_eq!(super::security_password("password: \"70617373\"\n").unwrap(),"70617373");
        assert_eq!(super::security_password("password: \"a\"b\"\n").unwrap(),"a\"b");
        assert_eq!(super::security_password("password: \n").unwrap(),"");
        let value="한글\\line\n끝";
        let hex:String=value.bytes().map(|b|format!("{b:02X}")).collect();
        assert_eq!(super::security_password(&format!("password: 0x{hex}  \"escaped hint\"\n")).unwrap(),value);
        assert!(super::security_password("password: 0xZZ\n").is_err());
        assert_eq!(super::security_arg("a\\b\"c"),"\"a\\\\b\\\"c\"");
    }
    #[test]
    fn groups_from_old_comma_field() {
        let mut h = super::Host { group: " 집, 회사,,집 ".into(), ..Default::default() };
        h.normalize_groups();
        assert_eq!(h.groups, ["집", "회사"]);
        assert_eq!(h.group, "집, 회사");
        h.groups = vec![];
        h.group = String::new();
        h.normalize_groups();
        assert!(h.groups.is_empty() && h.group.is_empty());
        let mut h = super::Host { group: "old".into(), groups: vec!["A".into(), "a".into(), " B ".into()], ..Default::default() };
        h.normalize_groups();
        assert_eq!((h.groups.clone(), h.group.as_str()), (vec!["A".to_string(), "B".to_string()], "A, B"));
    }

    use super::*;

    // (Keychain access needs the user's GUI login session, so this only runs on Windows.)
    #[cfg(windows)]
    #[test]
    fn credential_roundtrip() {
        let id = "test-0000";
        set_password(id, Some("p@ss 한글 ✓"));
        assert_eq!(get_password(id).as_deref(), Some("p@ss 한글 ✓"));
        set_password(id, None);
        assert_eq!(get_password(id), None);
    }

    #[cfg(windows)]
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
