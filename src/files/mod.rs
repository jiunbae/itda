//! File browsing and transfers, independent of remote screen sessions.
pub mod locations;
pub mod preview;
mod preview_cache;
mod preview_image;
#[cfg(any(target_os = "macos", windows))]
pub mod export;
#[cfg(any(target_os = "macos", windows))]
pub mod drag;
mod sftp;
mod transfer;
mod background;
pub use background::snapshot as background_jobs;
#[cfg(any(windows, test))]
pub mod vdrag;
use crate::{
    store::{Host, Login},
    tunnel::SshCfg,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use unicode_normalization::UnicodeNormalization;
use sftp::Sftp;
use std::{
    collections::HashMap,
    io,
    path::{Path, PathBuf},
    process::Child,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex, OnceLock,
    },
    time::UNIX_EPOCH,
};
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Options {
    pub method: String,
    /// Empty inherits the device account; "custom" uses a separate file login.
    pub account: String,
    pub host: String,
    pub port: u16,
    pub user: String,
    pub path: String,
    pub share: String,
    pub engine: String,
}
impl Default for Options {
    fn default() -> Self {
        Self {
            method: "auto".into(),
            account: String::new(),
            host: String::new(),
            port: 0,
            user: String::new(),
            path: String::new(),
            share: String::new(),
            engine: "auto".into(),
        }
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Entry {
    pub name: String,
    pub dir: bool,
    pub link: bool,
    pub size: u64,
    pub modified: u64,
    /// Ordinary POSIX permission bits (rwx for user, group, others: & 0o777, never setuid,
    /// setgid or sticky), when the side has them: a copy keeps scripts executable.
    /// None on Windows and from servers that don't report them.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mode: Option<u32>,
}
pub type Emit = Arc<dyn Fn(Value) + Send + Sync>;
#[derive(Default)]
pub struct Control {
    cancel: AtomicBool,
    child: Mutex<Vec<Arc<Mutex<Child>>>>,
    /// A Finder promise's write: counted through its operation queue, not as a job.
    promise: AtomicBool,
}
impl Control {
    pub fn check(&self) -> io::Result<()> {
        if self.cancel.load(Ordering::Relaxed) {
            Err(io::Error::new(
                io::ErrorKind::Interrupted,
                "전송을 취소했습니다",
            ))
        } else {
            Ok(())
        }
    }
    pub fn attach(&self, c: Arc<Mutex<Child>>) -> io::Result<()> {
        self.child.lock().unwrap().push(c);
        if self.cancel.load(Ordering::Relaxed) {
            self.stop();
            self.check()?;
        }
        Ok(())
    }
    pub fn stop(&self) {
        self.cancel.store(true, Ordering::Relaxed);
        for c in self.child.lock().unwrap().iter() {
            let _ = c.lock().unwrap().kill();
        }
    }
}
struct Connection {
    cfg: SshCfg,
    browse: Mutex<Sftp>,
    /// Previews' own SFTP session, kept between them (a new SSH login per preview cost about a
    /// second), and the preview working now.
    previews: PreviewSlot<Sftp>,
    control: Arc<Control>,
    home: String,
    options: Options,
    /// The remote is Windows: names it can't store are refused before anything is written.
    windows: bool,
}
/// One connection's previews: the idle session kept between them, and the preview working
/// now with its selection's number. Starting, finishing and stopping all change both under one
/// lock, so a stopped preview can never leave its (killed) session for the next.
struct PreviewSlot<T> {
    state: Mutex<(Option<T>, Option<(u64, Arc<Control>)>)>,
}
impl<T> Default for PreviewSlot<T> {
    fn default() -> Self {
        PreviewSlot { state: Mutex::new((None, None)) }
    }
}
impl<T> PreviewSlot<T> {
    /// Preview `generation` (the page's increasing request number) starts: one for an older
    /// selection still working is stopped. None when a newer one has already started (requests
    /// run on their own threads and may arrive out of order).
    fn begin(&self, generation: u64) -> Option<(Arc<Control>, Option<T>)> {
        let mut state = self.state.lock().unwrap();
        if let Some((working, control)) = &state.1 {
            if *working > generation {
                return None;
            }
            control.stop();
        }
        let control = Arc::new(Control::default());
        state.1 = Some((generation, control.clone()));
        Some((control, state.0.take()))
    }
    /// The preview ends with its session (None if it shouldn't be reused): kept for the next
    /// one only if this preview wasn't stopped.
    fn finish(&self, control: &Arc<Control>, session: Option<T>) {
        let mut state = self.state.lock().unwrap();
        if state.1.as_ref().is_some_and(|(_, c)| Arc::ptr_eq(c, control)) {
            state.1 = None;
        }
        if control.check().is_ok() {
            if let Some(session) = session {
                state.0 = Some(session);
            }
        }
    }
    /// The connection goes: stop the working preview, drop the idle session.
    fn stop(&self) {
        let mut state = self.state.lock().unwrap();
        if let Some((_, control)) = state.1.take() {
            control.stop();
        }
        state.0 = None;
    }
}
const NO_FILE: &str = "파일이 없습니다";
const NOT_A_FILE: &str = "일반 파일을 선택하세요";
static CONNECTIONS: OnceLock<Mutex<HashMap<String, Arc<Connection>>>> = OnceLock::new();
fn connections() -> &'static Mutex<HashMap<String, Arc<Connection>>> {
    CONNECTIONS.get_or_init(Default::default)
}
static CONNECTING: OnceLock<Mutex<HashMap<String, Arc<Control>>>> = OnceLock::new();
fn connecting() -> &'static Mutex<HashMap<String, Arc<Control>>> {
    CONNECTING.get_or_init(Default::default)
}
type Jobs = HashMap<String, (String, Arc<Control>)>;
static JOBS: OnceLock<Mutex<Jobs>> = OnceLock::new();
/// A running transfer of `id` owned by page `window`, registered for a test; dropping it
/// removes it again (the registry is the whole process's).
#[cfg(test)]
pub struct TestJob { pub job: String, control: Arc<Control> }
#[cfg(test)]
impl TestJob {
    pub fn stopped(&self) -> bool { self.control.check().is_err() }
    /// The job's thread ended (after a cancel, its cleanup).
    pub fn finish(&self) { jobs().lock().unwrap().remove(&self.job); }
}
#[cfg(test)]
impl Drop for TestJob { fn drop(&mut self) { self.finish(); } }
#[cfg(test)]
pub fn test_job(id: &str, window: &str) -> TestJob {
    let job = crate::store::new_id();
    let control = Arc::new(Control::default());
    jobs().lock().unwrap().insert(job.clone(), (scoped_id(id, &json!(window)), control.clone()));
    TestJob { job, control }
}
/// That transfer seen once by its page, which then closed.
#[cfg(test)]
pub fn test_adopt(window: &str, id: &str, job: &str) {
    let t = background::track(&json!({"names":["a"]}), json!(window), Arc::new(|_| {}));
    t(json!({"type":"files_job","id":id,"job":job,"state":"running"}));
    release_page(window);
}
/// Transfers and exports queued or running (also those whose file window was already
/// closed), and files being handed to Explorer from a drag: what quitting would cut off.
pub fn active_jobs() -> usize {
    let n = jobs().lock().unwrap().values().filter(|(_, c)| !c.promise.load(Ordering::Relaxed)).count();
    n + native_busy()
}
/// What the system file managers are still being given: Windows drag streams, and Finder
/// promises queued or being written (their writes are jobs too, counted only here).
fn native_busy() -> usize {
    let streams = { #[cfg(windows)] { vdrag::active_streams() } #[cfg(not(windows))] { 0 } };
    streams + PROMISE_BUSY.get().map_or(0, |f| f())
}
/// Set by the Mac app: how many Finder promise operations are queued or running.
pub static PROMISE_BUSY: OnceLock<fn() -> usize> = OnceLock::new();
thread_local! {
    /// This thread is writing a Finder promise (files_mac sets it around export::write).
    pub static PROMISE_WRITE: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}
/// Stop every transfer, export and drag stream; each cleans up (staged files, sessions) and
/// leaves `active_jobs` as it ends.
pub fn stop_jobs() {
    for (_, c) in jobs().lock().unwrap().values() {
        c.stop();
    }
    #[cfg(windows)]
    vdrag::stop_all();
}
/// Whether new transfers may start: not while Itda is quitting or installing an update.
#[cfg(not(test))]
static ACCEPTING: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(true);
#[cfg(not(test))]
fn set_accepting(on: bool) { ACCEPTING.store(on, std::sync::atomic::Ordering::SeqCst) }
// Tests run side by side: each test thread has its own (a quit test must not refuse
// another test's transfers).
#[cfg(test)]
thread_local! { static ACCEPTING_HERE: std::cell::Cell<bool> = const { std::cell::Cell::new(true) }; }
#[cfg(test)]
fn set_accepting(on: bool) { ACCEPTING_HERE.with(|a| a.set(on)) }
pub(crate) const CLOSING: &str = "Itda를 종료하는 중이라 새 전송을 시작하지 않았습니다";
/// Registers a job unless Itda is closing; checked and inserted under the registry's lock, so
/// a quit that closed admissions and then found nothing running can't miss one.
fn admit(job: &str, key: &str, control: &Arc<Control>) -> bool {
    let mut all = jobs().lock().unwrap();
    if !accepting() { return false; }
    all.insert(job.to_string(), (key.to_string(), control.clone()));
    true
}
/// Registers a drag stream (a whole drag, or one more file read) unless Itda is closing; one
/// step, like `admit`, so a closing count can't miss it.
#[cfg(any(windows, test))]
pub(crate) fn admit_stream(control: &Arc<Control>) -> Option<u64> {
    let _all = jobs().lock().unwrap();
    accepting().then(|| vdrag::register(control))
}
/// Stop admitting new transfers; what is running then (the decision for quitting now).
pub fn close_admissions() -> usize {
    let n = {
        let all = jobs().lock().unwrap();
        set_accepting(false);
        all.values().filter(|(_, c)| !c.promise.load(Ordering::Relaxed)).count()
    };
    n + native_busy()
}
pub fn open_admissions() {
    set_accepting(true);
}
pub fn accepting() -> bool {
    #[cfg(not(test))]
    return ACCEPTING.load(std::sync::atomic::Ordering::SeqCst);
    #[cfg(test)]
    return ACCEPTING_HERE.with(|a| a.get());
}
fn jobs() -> &'static Mutex<Jobs> {
    JOBS.get_or_init(Default::default)
}
fn error(s: impl Into<String>) -> io::Error {
    io::Error::other(s.into())
}
pub fn name(n: &str) -> io::Result<()> {
    if n.is_empty()
        || n == "."
        || n == ".."
        || n.chars().any(|c| c.is_control() || c == '/' || c == '\\')
    {
        return Err(error(
            "파일 이름에 경로 구분자나 제어 문자를 넣을 수 없습니다",
        ));
    }
    // On Windows, "C:x" joined to a folder replaces it (drive-relative) and "a:b" names a
    // stream of "a": a remote name like that would be written outside the target folder.
    if cfg!(windows) && !windows_name(n) {
        return Err(error("Windows에서 쓸 수 없는 파일 이름입니다"));
    }
    Ok(())
}

/// Windows stores names as provided, including decomposed Hangul from macOS.
/// Normalize destination components only; source lookups must retain the exact name.
fn destination_name(n: &str, windows: bool) -> String {
    if windows { n.nfc().collect() } else { n.to_string() }
}
fn windows_name_key(n: &str) -> String { destination_name(n, true).to_lowercase() }
fn destination_leaf(path: &str, windows: bool) -> String {
    let at = path.rfind(['/', '\\']).map_or(0, |at|at+1);
    format!("{}{}", &path[..at],destination_name(&path[at..],windows))
}

/// A name about to be written on a Windows side, refused with the item it names.
fn windows_target(n: &str, shown: &str) -> io::Result<()> {
    if windows_name(&destination_name(n, true)) {
        Ok(())
    } else {
        Err(error(format!("Windows에서 쓸 수 없는 파일 이름입니다: {shown}")))
    }
}

/// A remote home like "/C:/Users/me" (Win32-OpenSSH) or "C:/Users/me".
fn windows_root(p: &str) -> bool {
    let b = p.strip_prefix('/').unwrap_or(p).as_bytes();
    b.len() >= 2 && b[0].is_ascii_alphabetic() && b[1] == b':'
}

/// A remote path as typed: without Windows' \\?\ prefix, which the SFTP server refuses,
/// and with "~" for the connection's home.
fn remote_path(p: &str, home: &str) -> String {
    if p == "~" {
        home.to_string()
    } else if let Some(rest) = p.strip_prefix("~/") {
        format!("{}/{rest}", home.trim_end_matches('/'))
    } else {
        plain(p)
    }
}

/// Can Windows store a file under this name as-is: no <>:"|?*, no trailing dot or space,
/// not a device name (CON, NUL, COM1…, with or without an extension).
fn windows_name(n: &str) -> bool {
    if n.chars().any(|c| matches!(c, '<' | '>' | ':' | '"' | '|' | '?' | '*')) || n.ends_with(['.', ' ']) {
        return false;
    }
    let stem = n.split('.').next().unwrap_or("").trim_end().to_ascii_uppercase();
    let device = matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL" | "CONIN$" | "CONOUT$")
        // Win32 also recognizes superscript 1/2/3 as device-port digits.
        || matches!(stem.as_str(), "COM¹" | "COM²" | "COM³" | "LPT¹" | "LPT²" | "LPT³")
        || ((stem.starts_with("COM") || stem.starts_with("LPT"))
            && stem.len() == 4
            && stem.as_bytes()[3].is_ascii_digit());
    !device
}

/// What a remote file may be opened as straight from the remote side (a preview): common
/// text, documents, images, audio and video. Anything else, scripts and unknown types
/// included, is copied to this device first and opened knowingly; a list of what to refuse
/// would always miss some interpreter's extension (.pyw, .hta, .terminal…).
const PREVIEWABLE: &[&str] = &[
    // text and documents
    "txt", "md", "csv", "tsv", "log", "json", "rtf", "pdf", "doc", "docx", "xls", "xlsx", "ppt", "pptx",
    "odt", "ods", "odp", "epub",
    // images
    "png", "jpg", "jpeg", "gif", "bmp", "webp", "heic", "heif", "tif", "tiff", "ico", "avif",
    // audio
    "mp3", "m4a", "aac", "wav", "flac", "ogg", "opus", "aif", "aiff",
    // video
    "mp4", "m4v", "mov", "avi", "mkv", "webm",
];

fn previewable(file: &str) -> bool {
    Path::new(file)
        .extension()
        .and_then(|s| s.to_str())
        .is_some_and(|e| PREVIEWABLE.contains(&e.to_ascii_lowercase().as_str()))
}

/// Mark a file fetched from another device as downloaded, so the system's own checks
/// (Gatekeeper on macOS, SmartScreen / Office Protected View on Windows) treat it so.
fn mark_downloaded(path: &Path) {
    #[cfg(target_os = "macos")]
    {
        let secs = std::time::SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs());
        let _ = std::process::Command::new("/usr/bin/xattr")
            .args(["-w", "com.apple.quarantine", &format!("0081;{secs:x};Itda;"), "--"])
            .arg(path)
            .status();
    }
    #[cfg(windows)]
    {
        let mut ads = path.as_os_str().to_owned();
        ads.push(":Zone.Identifier");
        let _ = std::fs::write(ads, "[ZoneTransfer]\r\nZoneId=3\r\n");
    }
    #[cfg(not(any(target_os = "macos", windows)))]
    let _ = path;
}

/// A local path made absolute, without Windows' \\?\ prefix: canonicalize adds it, and
/// then '/' stops working as a separator and the prefix shows in the path bar.
pub(crate) fn canonical(p: &str) -> io::Result<String> {
    Ok(plain(&std::fs::canonicalize(p)?.to_string_lossy()))
}

fn plain(s: &str) -> String {
    if let Some(rest) = s.strip_prefix(r"\\?\UNC\") {
        format!(r"\\{rest}")
    } else if let Some(rest) = s.strip_prefix(r"\\?\") {
        rest.to_string()
    } else {
        s.to_string()
    }
}
fn join(parent: &str, child: &str) -> io::Result<String> {
    name(child)?;
    Ok(format!("{}/{}", parent.trim_end_matches('/'), child))
}
pub fn local_home() -> PathBuf {
    std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" })
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir)
}
fn local_mode(m: &std::fs::Metadata) -> Option<u32> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        Some(m.permissions().mode() & 0o777)
    }
    #[cfg(not(unix))]
    {
        let _ = m;
        None
    }
}
fn local_entry(p: &Path) -> io::Result<Entry> {
    local_entry_with(p, std::fs::symlink_metadata(p)?)
}
fn local_entry_with(p: &Path, m: std::fs::Metadata) -> io::Result<Entry> {
    Ok(Entry {
        name: p
            .file_name()
            .and_then(|n| n.to_str())
            .ok_or_else(|| error("파일 이름을 표시할 수 없습니다"))?
            .into(),
        dir: m.is_dir(),
        link: m.file_type().is_symlink(),
        size: m.len(),
        modified: m
            .modified()
            .ok()
            .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
            .map_or(0, |t| t.as_secs()),
        mode: local_mode(&m),
    })
}
/// Windows' "This PC": the drives that are there (an empty optical drive is not).
fn drives() -> Vec<Entry> {
    if !cfg!(windows) {
        return Vec::new();
    }
    (b'A'..=b'Z')
        .map(|l| format!("{}:", l as char))
        .filter(|d| Path::new(&format!("{d}\\")).is_dir())
        .map(|name| Entry { name, dir: true, link: false, size: 0, modified: 0, mode: None })
        .collect()
}
/// The folder's entries. One that can't be shown or copied (a name that isn't UTF-8, or
/// that name() refuses, like macOS's "Icon\r") is left out rather than failing the folder.
fn local_list(p: &str) -> io::Result<Vec<Entry>> {
    let mut out = Vec::new();
    for e in std::fs::read_dir(p)? {
        // The directory scan's own metadata (not followed through links, like
        // symlink_metadata): on Windows it comes with the scan instead of opening each file,
        // which on a network drive was a round trip per file.
        let e = e?;
        if let Ok(entry) = e.metadata().and_then(|m| local_entry_with(&e.path(), m)) {
            if name(&entry.name).is_ok() {
                out.push(entry);
            }
        }
    }
    Ok(out)
}
fn get(v: &Value, k: &str) -> String {
    v[k].as_str().unwrap_or("").into()
}
pub fn password_key(h: &Host) -> String {
    if h.files.account.is_empty() {
        format!("files-{}", h.id)
    } else {
        format!("files-{}-{}-{}", h.id, h.files.account, h.files.user)
    }
}
fn ssh_addr(host: &str, port: u16) -> String {
    if host.contains(':') { format!("[{host}]:{port}") } else { format!("{host}:{port}") }
}

fn cfg(h: &Host, login: &Login, password: Option<String>) -> io::Result<SshCfg> {
    let o = &h.files;
    let (default, _) = crate::tunnel::split_host(&h.host, 0);
    let host = if o.host.is_empty() {
        default
    } else {
        o.host.clone()
    };
    let user = if o.user.is_empty() {
        login.user.clone()
    } else {
        o.user.clone()
    };
    if user.trim().is_empty() {
        return Err(error(
            "파일에 접근할 기기의 계정 이름을 입력하세요. Itda 화면 공유 암호와는 별도입니다",
        ));
    }
    let pass = password
        .or_else(|| crate::store::get_password(&password_key(h)))
        .or_else(|| (user == login.user).then(|| login.pass.clone()).flatten())
        .unwrap_or_default();
    let c = SshCfg {
        host,
        user: user.clone(),
        pass,
        port: if o.port == 0 {
            if h.ssh.port == 0 {
                22
            } else {
                h.ssh.port
            }
        } else {
            o.port
        },
        vnc_port: 0,
        key: if user == login.user {
            login.ssh_key.clone()
        } else {
            String::new()
        },
        key_pass: if user == login.user {
            login.key_pass.clone().unwrap_or_default()
        } else {
            String::new()
        },
        key_alias: String::new(),
    };
    // The saved address not answering on the file port but the LAN one doing so: go there,
    // as long as it proves to be the same device (its trusted SSH key).
    let mut c = c;
    if o.host.is_empty() && !h.lan.trim().is_empty() && !crate::store::answers(&ssh_addr(&c.host, c.port)) {
        let lan = h.lan.trim().to_string();
        if crate::store::answers(&ssh_addr(&lan, c.port)) {
            crate::log!("files {}: not answering, using its LAN address {lan}", c.host);
            c.key_alias = std::mem::replace(&mut c.host, lan);
        }
    }
    crate::tunnel::ssh_target(&c)?;
    Ok(c)
}
// One copied remote selection may outlive its window. Retain only its connection metadata;
// transfers open fresh SFTP streams with their own cancellation control. The clipboard ID
// prevents a later Copy from lending its credentials to an older paste or a drag operation.
struct ClipboardSource {
    cid: String,
    key: String,
    connection: Arc<Connection>,
}
static CLIPBOARD_SOURCE: Mutex<Option<ClipboardSource>> = Mutex::new(None);
pub fn remember_clipboard_source(clip: &Value) {
    let candidate = (|| {
        if clip["side"] != "remote" { return None; }
        let cid = clip["cid"].as_str()?.to_string();
        let key = scoped_id(clip["host"].as_str()?, &clip["window"]);
        Some((cid, key))
    })();
    let mut kept = CLIPBOARD_SOURCE.lock().unwrap();
    if let Some((cid, key)) = candidate {
        if kept.as_ref().is_some_and(|k| k.cid == cid && k.key == key) { return; }
        *kept = connection(&key).ok().map(|connection| ClipboardSource { cid, key, connection });
    } else { *kept = None; }
}
fn transfer_source(key: &str, clipboard: Option<&str>, drag_token: Option<&str>) -> io::Result<Arc<Connection>> {
    #[cfg(any(target_os = "macos", windows))]
    if let Some(token) = drag_token { return drag::source(key, token).ok_or_else(|| io::Error::other("끌어 놓은 파일의 연결이 만료되었습니다. 다시 끌어 놓아 주세요")); }
    #[cfg(not(any(target_os = "macos", windows)))]
    let _ = drag_token;
    if let Some(cid) = clipboard {
        if let Some(kept) = CLIPBOARD_SOURCE.lock().unwrap().as_ref().filter(|k| k.cid == cid && k.key == key) {
            return Ok(kept.connection.clone());
        }
    }
    connection(key)
}

/// Isolate browsing, cancellation and transfer jobs for each file-browser page.
pub fn scoped_id(id: &str, window: &Value) -> String {
    match window.as_str().filter(|s| !s.is_empty()) {
        Some(window) => format!("{id}\x1f{window}"),
        None => id.to_string(),
    }
}
pub fn route_event(v: &mut Value) {
    if let Some(id) = v["id"].as_str().map(str::to_string) {
        if let Some((host, window)) = id.rsplit_once('\x1f') {
            v["id"] = json!(host); v["window"] = json!(window);
        }
    }
}
fn retired() -> &'static Mutex<std::collections::HashSet<String>> {
    static KEYS: OnceLock<Mutex<std::collections::HashSet<String>>> = OnceLock::new();
    KEYS.get_or_init(Default::default)
}
/// Release an unused browse session; active/queued transfers keep it until completion.
pub(super) fn cleanup_retired(key: &str) {
    let mut retired = retired().lock().unwrap();
    if retired.contains(key) && !jobs().lock().unwrap().values().any(|(owner,_)| owner == key) {
        disconnect(key); retired.remove(key);
    }
}
fn retire(key: String) {
    retired().lock().unwrap().insert(key.clone()); cleanup_retired(&key);
}
pub fn release_page(page: &str) {
    preview::revoke_page(page);
    preview_cache::release_page(page);
    if page.is_empty() { return; }
    // Its transfers keep running, now the app's (shown and cancellable from any Itda page).
    background::adopt(page);
    let suffix = format!("\x1f{page}");
    let mut keys: Vec<String> = connections().lock().unwrap().keys().chain(connecting().lock().unwrap().keys()).filter(|key| key.ends_with(&suffix)).cloned().collect();
    keys.extend(jobs().lock().unwrap().values().filter(|(key,_)| key.ends_with(&suffix)).map(|(key,_)| key.clone()));
    keys.sort(); keys.dedup(); for key in keys { retire(key); }
}
pub fn disconnect(id: &str) {
    preview::revoke_connection(id);
    preview_cache::invalidate(id);
    if !id.contains('\x1f') {
        let prefix = format!("{id}\x1f");
        // The device itself is going (deleted, or its files disconnected): a copied selection
        // from it no longer keeps its connection open. (A closed window alone keeps it.)
        let mut kept = CLIPBOARD_SOURCE.lock().unwrap();
        if kept.as_ref().is_some_and(|k| k.key == id || k.key.starts_with(&prefix)) {
            *kept = None;
        }
        drop(kept);
        let mut keys: Vec<String> = connections().lock().unwrap().keys().chain(connecting().lock().unwrap().keys()).filter(|key| key.starts_with(&prefix)).cloned().collect();
        keys.extend(jobs().lock().unwrap().values().filter(|(key,_)| key.starts_with(&prefix)).map(|(key,_)| key.clone()));
        keys.sort(); keys.dedup();
        for key in keys { disconnect(&key); }
    }
    if let Some(c) = connecting().lock().unwrap().remove(id) {
        c.stop();
    }
    if let Some(c) = connections().lock().unwrap().remove(id) {
        c.control.stop();
        // A preview downloading now stops too, and the idle session goes.
        c.previews.stop();
    }
    for (host, c) in jobs().lock().unwrap().values() {
        if host == id {
            c.stop();
        }
    }
}
/// All operations run outside the UI thread. No file credentials are returned to JavaScript.
pub fn dispatch(v: Value, host: Host, login: Login, emit: Emit) {
    let op = get(&v, "op");
    let id = host.id.clone();
    let key = scoped_id(&id, &v["window"]);
    let window = v["window"].clone();
    if !["cancel", "cancel_background", "dismiss_background", "release"].contains(&op.as_str()) {
        background::in_use(window.as_str().unwrap_or(""));
    }
    if op == "transfer" {
        transfer::start(v.clone(), host, background::track(&v, window, emit));
        return;
    }
    // A transfer whose window closed belongs to the app: any page may stop or dismiss it.
    // A job whose page is open stays that page's own (below).
    if op == "cancel_background" {
        let job = get(&v, "job");
        if background::is_orphan(&job) {
            if let Some((_, c)) = jobs().lock().unwrap().get(&job) { c.stop(); }
        }
        return;
    }
    if op == "dismiss_background" { background::dismiss(&get(&v, "job")); return; }
    let emit: Emit = Arc::new(move |mut m| { m["window"] = window.clone(); emit(m); });
    if op == "release" { retire(key); return; }
    if op == "connect" { retired().lock().unwrap().remove(&key); }
    if op == "cancel" {
        if let Some((owner, c)) = jobs().lock().unwrap().get(&get(&v, "job")) {
            if owner == &key { c.stop(); }
        }
        return;
    }
    if op == "disconnect" {
        disconnect(&key);
        return;
    }
    if mutates_files(&op) { preview_cache::invalidate_all(); }
    let preview_ticket = if op == "preview" && crate::home::binary_previews() {
        preview::begin(window_page(&v), &key).map(Some)
    } else { Ok(None) };
    std::thread::spawn(move || {
        let result = (|| -> io::Result<Value> {
            match op.as_str() {
                "connect" => {
                    if v["reuse"] == true {
                        if let Ok(c) = connection(&key) {
                            if c.options == host.files && c.control.check().is_ok() {
                                let entries = c.browse.lock().unwrap().list(&c.home)?;
                                let local = local_home().to_string_lossy().into_owned();
                                return Ok(
                                    json!({"path":c.home,"entries":entries,"local_path":local,"local_entries":local_list(&local)?,"rsync":transfer::rsync_available()}),
                                );
                            }
                        }
                    }
                    disconnect(&key);
                    let control = Arc::new(Control::default());
                    connecting()
                        .lock()
                        .unwrap()
                        .insert(key.clone(), control.clone());
                    let c = cfg(
                        &host,
                        &login,
                        v["password"]
                            .as_str()
                            .filter(|p| !p.is_empty())
                            .map(str::to_string),
                    )?;
                    let mut s = Sftp::connect(&c, &control)?;
                    let root = s.realpath(&if host.files.path.is_empty() {
                        ".".to_string()
                    } else {
                        remote_path(&host.files.path, ".")
                    })?;
                    let entries = s.list(&root)?;
                    control.check()?;
                    if v["save_password"] == true {
                        if let Some(p) = v["password"].as_str().filter(|p| !p.is_empty()) {
                            crate::store::set_password(&password_key(&host), Some(p));
                        }
                    }
                    let mut sessions = connections().lock().unwrap();
                    // Closing a page can cancel a connect after its first check.
                    // Check under the insertion lock so disconnect cannot miss it.
                    control.check()?;
                    sessions.insert(
                        key.clone(),
                        Arc::new(Connection {
                            cfg: c,
                            browse: Mutex::new(s),
                            previews: PreviewSlot::default(),
                            control,
                            windows: host.os == "windows" || windows_root(&root),
                            home: root.clone(),
                            options: host.files.clone(),
                        }),
                    );
                    drop(sessions);
                    let local = local_home().to_string_lossy().into_owned();
                    Ok(
                        json!({"path":root,"entries":entries,"local_path":local,"local_entries":local_list(&local)?,"rsync":transfer::rsync_available()}),
                    )
                }
                "preview" => {
                    let ticket = preview_ticket?;
                    // Return bounded inert content; never launch an application or render HTML/SVG.
                    let p = get(&v, "path");
                    let ext = Path::new(&p)
                        .extension()
                        .and_then(|x| x.to_str())
                        .unwrap_or("")
                        .to_ascii_lowercase();
                    let mime = match ext.as_str() {
                        "txt" | "md" | "csv" | "tsv" | "log" | "json" => "text/plain",
                        "png" => "image/png",
                        "jpg" | "jpeg" => "image/jpeg",
                        "gif" => "image/gif",
                        "webp" => "image/webp",
                        "bmp" => "image/bmp",
                        "avif" => "image/avif",
                        "pdf" if cfg!(target_os = "macos") => "application/pdf",
                        "mp4" | "m4v" => "video/mp4",
                        "webm" => "video/webm",
                        "mp3" => "audio/mpeg",
                        "wav" => "audio/wav",
                        "ogg" => "audio/ogg",
                        _ => return Ok(json!({"unsupported":true})),
                    };
                    let limit = if mime == "text/plain" {
                        512 * 1024
                    } else {
                        16 * 1024 * 1024
                    };
                    let mut cache_key = None;
                    let _work;
                    let bytes = if v["side"] == "local" {
                        _work = preview_image::work(ticket.as_ref())?;
                        use std::io::Read;
                        let canonical = std::fs::canonicalize(&p)?;
                        let mut file = std::fs::File::open(&canonical)?;
                        let metadata = file.metadata()?;
                        if !metadata.is_file() { return Err(error("일반 파일을 선택하세요")); }
                        if metadata.len() > limit { return Ok(json!({"too_large":true})); }
                        if mime != "text/plain" {
                            if let Ok(modified) = metadata.modified().and_then(|m| m.duration_since(UNIX_EPOCH).map_err(io::Error::other)) {
                                let k = preview_cache::key(&key, canonical.to_string_lossy().into_owned(), metadata.len(), modified.as_nanos());
                                if let Some(cached) = preview_cache::get(&k) { return preview_prepared(ticket.as_ref(), cached); }
                                cache_key = Some(k);
                            }
                        }
                        let mut bytes = Vec::new(); file.by_ref().take(limit + 1).read_to_end(&mut bytes)?; bytes
                    } else {
                        let c = connection(&key)?;
                        let p = remote_path(&p, &c.home);
                        // A preview of an older selection still downloading stops; this one,
                        // if a newer selection already asked, doesn't run at all.
                        let generation = v["seq"].as_u64().unwrap_or(u64::MAX);
                        let (control, kept, permit) = preview_remote_begin(&c.previews, generation, ticket.as_ref())?;
                        _work = permit;
                        let started = std::time::Instant::now();
                        let reused = kept.is_some();
                        let mut session = kept;
                        let mut cached_remote = None;
                        let result = (|| {
                            control.check()?;
                            let s = match session.take() {
                                Some(s) => {
                                    s.bind(&control)?;
                                    session.insert(s)
                                }
                                None => session.insert(Sftp::connect(&c.cfg, &control)?),
                            };
                            let entry = s.stat(&p)?.ok_or_else(|| error(NO_FILE))?;
                            if entry.dir || entry.link {
                                return Err(error(NOT_A_FILE));
                            }
                            if entry.size > limit {
                                return Ok(None);
                            }
                            if mime != "text/plain" && entry.modified != 0 {
                                // Preserve LSTAT refusal of a symlink leaf before canonicalizing.
                                if let Ok(canonical) = s.realpath(&p) {
                                    let k = preview_cache::key(&key, canonical, entry.size, entry.modified as u128 * 1_000_000_000);
                                    if let Some(cached) = preview_cache::get(&k) {
                                        cached_remote = Some(cached);
                                        return Ok(Some(Vec::new()));
                                    }
                                    cache_key = Some(k);
                                }
                            }
                            let mut bytes = Vec::new();
                            s.download(&p, entry.size, &mut bytes, &control, |_| {})?;
                            Ok(Some(bytes))
                        })();
                        // Only a refusal about the file itself leaves the session usable;
                        // after any other failure it isn't kept (nor after a stop: finish
                        // decides that under the same lock as a newer preview's stop).
                        let usable = match &result {
                            Ok(_) => true,
                            Err(e) => [NOT_A_FILE, NO_FILE].contains(&e.to_string().as_str()),
                        };
                        c.previews.finish(&control, session.filter(|_| usable));
                        if cached_remote.is_none() { if let Ok(Some(bytes)) = &result {
                            crate::log!("files: preview {} bytes in {} ms ({} session)", bytes.len(), started.elapsed().as_millis(), if reused { "kept" } else { "new" });
                        }
                        }
                        let bytes = match result? {
                            Some(bytes) => bytes,
                            None => return Ok(json!({"too_large":true})),
                        };
                        if let Some(cached) = cached_remote { return preview_prepared(ticket.as_ref(), cached); }
                        bytes
                    };
                    if bytes.len() as u64 > limit { return Ok(json!({"too_large":true})); }
                    if mime == "text/plain" { return Ok(json!({"mime":mime,"text":String::from_utf8_lossy(&bytes)})); }
                    if ticket.as_ref().is_some_and(|t| !preview::current(t)) { return Err(io::Error::new(io::ErrorKind::Interrupted, "이전 미리보기입니다")); }
                    let prepared = if mime == "application/pdf" {
                        #[cfg(target_os = "macos")]
                        { let Some(png) = pdf_preview(&bytes, ticket.as_ref())? else { return Ok(json!({"unsupported":true})); };
                          preview_cache::Prepared {bytes:Arc::new(png),mime:"image/png".into(),first_page:true} }
                        #[cfg(not(target_os = "macos"))]
                        { return Ok(json!({"unsupported":true})); }
                    } else { preview_image::prepare(bytes, mime, ticket.as_ref())? };
                    if ticket.as_ref().is_none_or(preview::current) {
                        if let Some(k) = cache_key { preview_cache::insert(k, prepared.clone()); }
                    }
                    preview_prepared(ticket.as_ref(), prepared)
                }

                "reveal_folder" => {
                    if v["side"] != "local" {
                        return Err(error("이 컴퓨터의 폴더를 선택하세요"));
                    }
                    let p = local_folder(&get(&v, "path"))?;
                    #[cfg(target_os = "macos")]
                    {
                        // Reveal packages in Finder; opening a .app directory would launch it.
                        std::process::Command::new("/usr/bin/open").args(["-R", "--", &p]).spawn()?;
                    }
                    #[cfg(windows)]
                    open_local(&p)?;
                    Ok(json!({}))
                }
                "open" => {
                    let p = get(&v, "path");
                    if v["side"] == "local" {
                        open_local(&p)?;
                    } else {
                        if !previewable(&p) {
                            return Err(error(
                                "이 형식은 원격에서 바로 열 수 없습니다. 내 기기로 복사한 뒤 열어 주세요",
                            ));
                        }
                        let c = connection(&key)?;
                        let p = remote_path(&p, &c.home);
                        let control = Control::default();
                        let mut s = Sftp::connect(&c.cfg, &control)?;
                        let entry = s.stat(&p)?.ok_or_else(|| error("파일이 없습니다"))?;
                        if entry.dir || entry.link {
                            return Err(error("일반 파일을 선택하세요"));
                        }
                        let root = crate::store::dir()
                            .join("file-previews")
                            .join(crate::store::new_id());
                        std::fs::create_dir_all(&root)?;
                        name(&entry.name)?;
                        let target = root.join(&entry.name);
                        let mut opts = std::fs::OpenOptions::new();
                        opts.write(true).create_new(true);
                        #[cfg(unix)]
                        {
                            use std::os::unix::fs::OpenOptionsExt;
                            opts.mode(0o600);
                        }
                        let mut file = opts.open(&target)?;
                        s.download(&p, entry.size, &mut file, &control, |_| {})?;
                        file.sync_all()?;
                        drop(file);
                        mark_downloaded(&target);
                        let mut permissions = std::fs::metadata(&target)?.permissions();
                        permissions.set_readonly(true);
                        std::fs::set_permissions(&target, permissions)?;
                        open_local(
                            target
                                .to_str()
                                .ok_or_else(|| error("파일 경로를 표시할 수 없습니다"))?,
                        )?;
                    }
                    Ok(json!({"preview":v["side"]=="remote"}))
                }
                // This computer's home, for a window that has no device connected yet.
                "local_home" => {
                    let home = local_home().to_string_lossy().into_owned();
                    Ok(json!({"path":home,"entries":local_list(&home)?}))
                }
                "list" => {
                    let p = get(&v, "path");
                    if v["side"] == "local" && p.is_empty() && cfg!(windows) {
                        Ok(json!({"path":"","entries":drives()}))
                    } else if v["side"] == "local" {
                        Ok(
                            json!({"path":canonical(&p)?,"entries":local_list(&p)?}),
                        )
                    } else {
                        let c = connection(&key)?;
                        let mut s = c.browse.lock().unwrap();
                        let p = s.realpath(&if p.is_empty() {
                            c.home.clone()
                        } else {
                            remote_path(&p, &c.home)
                        })?;
                        Ok(json!({"path":p,"entries":s.list(&p)?}))
                    }
                }
                "undo" => {
                    let from = get(&v, "from");
                    let to = get(&v, "to");
                    if v["side"] == "local" {
                        rename_local(&from, &to, false)?;
                    } else {
                        let c = connection(&key)?;
                        c.browse.lock().unwrap().rename(&from, &to, false)?;
                    }
                    Ok(json!({"undo":{"side":v["side"],"from":to,"to":from}}))
                }
                "mkdir" | "rename" | "trash" => {
                    let p = get(&v, "path");
                    let n = get(&v, "name");
                    name(&n)?;
                    // "" is Windows' This PC (the drive list), never a folder to change.
                    if v["side"] == "local" && p.is_empty() {
                        return Err(error("내 PC에서는 드라이브를 열 수만 있습니다"));
                    }
                    let mut undo = Value::Null;
                    if v["side"] == "local" {
                        let source = Path::new(&p).join(&n);
                        let source = source
                            .to_str()
                            .ok_or_else(|| error("파일 경로를 표시할 수 없습니다"))?;
                        if op == "mkdir" {
                            std::fs::create_dir(source)?;
                        } else {
                            let target = if op == "rename" {
                                let new = get(&v, "new_name");
                                name(&new)?;
                                Path::new(&p).join(new)
                            } else {
                                let trash = local_home().join(".itda-trash");
                                std::fs::create_dir_all(&trash)?;
                                if std::fs::symlink_metadata(&trash)?.file_type().is_symlink() {
                                    return Err(error(
                                        "보관함이 심볼릭 링크입니다. 다른 폴더를 사용해 주세요",
                                    ));
                                }
                                trash.join(format!("{}-{n}", crate::store::new_id()))
                            };
                            let target = target
                                .to_str()
                                .ok_or_else(|| error("파일 경로를 표시할 수 없습니다"))?;
                            rename_local(source, target, false)?;
                            undo = json!({"side":"local","from":target,"to":source});
                        }
                    } else {
                        let c = connection(&key)?;
                        let mut s = c.browse.lock().unwrap();
                        let p = remote_path(&p, &c.home);
                        let source = join(&p, &n)?;
                        // Refused here with a clear reason rather than by the server's bare
                        // status: names Windows can't store (it would drop a trailing dot or
                        // make a device-named file) and a name that is already taken.
                        let taken = |s: &mut Sftp, path: &str, shown: &str| -> io::Result<()> {
                            if s.stat(path)?.is_some() {
                                return Err(error(format!("같은 이름의 항목이 있습니다: {shown}")));
                            }
                            Ok(())
                        };
                        if op == "mkdir" {
                            if c.windows {
                                windows_target(&n, &n)?;
                            }
                            taken(&mut s, &source, &n)?;
                            s.mkdir(&source)?;
                        } else {
                            let target = if op == "rename" {
                                let new = get(&v, "new_name");
                                let target = join(&p, &new)?;
                                if c.windows {
                                    windows_target(&new, &new)?;
                                }
                                // A case-only change names the same item on a case-insensitive
                                // remote; Sftp::rename handles that.
                                if new.to_lowercase() != n.to_lowercase() {
                                    taken(&mut s, &target, &new)?;
                                }
                                target
                            } else {
                                let trash = join(&c.home, ".itda-trash")?;
                                match s.stat(&trash)? {
                                    None => s.mkdir(&trash)?,
                                    Some(e) if !e.dir || e.link => {
                                        return Err(error("보관함 폴더를 사용할 수 없습니다"))
                                    }
                                    _ => {}
                                }
                                join(&trash, &format!("{}-{n}", crate::store::new_id()))?
                            };
                            s.rename(&source, &target, false)?;
                            undo = json!({"side":"remote","from":target,"to":source});
                        }
                    }
                    Ok(json!({"undo":undo}))
                }
                _ => Err(error("지원하지 않는 파일 작업입니다")),
            }
        })();
        if mutates_files(&op) { preview_cache::invalidate_all(); }
        if op == "connect"
            && host.files.method == "auto"
            && result.as_ref().err().is_some_and(|e| {
                matches!(
                    e.kind(),
                    io::ErrorKind::ConnectionRefused
                        | io::ErrorKind::NotFound
                        | io::ErrorKind::TimedOut
                        | io::ErrorKind::AddrNotAvailable
                )
            })
        {
            use std::net::{TcpStream, ToSocketAddrs};
            let (address, _) = crate::tunnel::split_host(
                if host.files.host.is_empty() {
                    &host.host
                } else {
                    &host.files.host
                },
                0,
            );
            let available = (address.as_str(), 445)
                .to_socket_addrs()
                .ok()
                .is_some_and(|mut a| {
                    a.any(|a| {
                        TcpStream::connect_timeout(&a, std::time::Duration::from_millis(700))
                            .is_ok()
                    })
                });
            if available {
                let target = if address.contains(':') {
                    format!("smb://[{address}]")
                } else {
                    format!("smb://{address}")
                };
                let native = open_native(&target);
                emit(
                    json!({"type":"files_result","id":id,"op":"native","ok":native.is_ok(),"error":native.err().map(|e|e.to_string())}),
                );
                return;
            }
        }
        emit(
            json!({"type":"files_result","id":id,"op":op,"seq":v["seq"],"side":v["side"],"ok":result.is_ok(),"data":result.as_ref().ok(),"error":result.err().map(|e|e.to_string())}),
        );
    });
}
/// Render only the first PDF page to inert pixels; PDF plugins in sandboxed frames are
/// unavailable in WebKit/WebView2. Keep the source private and remove both files afterward.
#[cfg(target_os = "macos")]
fn pdf_preview(bytes: &[u8], ticket: Option<&preview::Ticket>) -> io::Result<Option<Vec<u8>>> {
    use std::{process::{Command, Stdio}, time::{Duration, Instant}};
    use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
    use std::io::Write;
    let root = crate::store::dir().join("file-previews").join(crate::store::new_id());
    std::fs::create_dir_all(&root)?;
    let result = (|| {
        std::fs::set_permissions(&root, std::fs::Permissions::from_mode(0o700))?;
        let input = root.join("preview.pdf");
        let output = root.join("preview.png");
        std::fs::OpenOptions::new().write(true).create_new(true).mode(0o600).open(&input)?.write_all(bytes)?;
        let mut child = Command::new("/usr/bin/sips").args(["-s", "format", "png", "-Z", "1600"])
            .arg(&input).arg("--out").arg(&output).stdout(Stdio::null()).stderr(Stdio::null()).spawn()?;
        let started = Instant::now();
        loop {
            if ticket.is_some_and(|t| !preview::current(t)) {
                let _ = child.kill(); let _ = child.wait();
                return Err(io::Error::new(io::ErrorKind::Interrupted, "이전 미리보기입니다"));
            }
            if let Some(status) = child.try_wait()? {
                if !status.success() { return Ok(None); }
                break;
            }
            if started.elapsed() > Duration::from_secs(8) {
                let _ = child.kill(); let _ = child.wait();
                return Ok(None);
            }
            std::thread::sleep(Duration::from_millis(25));
        }
        let png = std::fs::read(output)?;
        if png.len() > 16 * 1024 * 1024 { return Ok(None); }
        Ok(Some(png))
    })();
    let _ = std::fs::remove_dir_all(root);
    result
}
fn connection(id: &str) -> io::Result<Arc<Connection>> {
    connections()
        .lock()
        .unwrap()
        .get(id)
        .cloned()
        .ok_or_else(|| error("먼저 파일 연결을 열어 주세요"))
}
pub fn native_target(host: &Host) -> io::Result<String> {
    if !host.files.share.is_empty() {
        smb_target(&host.files.share, cfg!(windows))?;
        return Ok(host.files.share.clone());
    }
    let (address, _) = crate::tunnel::split_host(
        if host.files.host.is_empty() {
            &host.host
        } else {
            &host.files.host
        },
        0,
    );
    let url = if address.contains(':') {
        format!("smb://[{address}]")
    } else {
        format!("smb://{address}")
    };
    smb_target(&url, cfg!(windows))?;
    Ok(url)
}
pub fn smb_target(address: &str, windows: bool) -> io::Result<String> {
    if address.chars().any(|c| c.is_control()) {
        return Err(error("주소에 제어 문자를 넣을 수 없습니다"));
    }
    let address = address.trim();
    let url = address
        .get(..6)
        .is_some_and(|p| p.eq_ignore_ascii_case("smb://"));
    let rest = if url {
        address[6..].to_string()
    } else if let Some(s) = address.strip_prefix("\\\\") {
        s.replace('\\', "/")
    } else {
        return Err(error(
            "smb://기기/공유폴더 또는 \\\\기기\\공유폴더 형식으로 입력하세요",
        ));
    };
    let (server, path) = rest.split_once('/').unwrap_or((&rest, ""));
    let ipv6 = server
        .strip_prefix('[')
        .and_then(|s| s.strip_suffix(']'))
        .and_then(|s| s.parse::<std::net::Ipv6Addr>().ok());
    if server.is_empty()
        || (ipv6.is_none()
            && !server
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'-' | b'_')))
    {
        return Err(error(
            "기기 주소를 확인하세요. 계정과 암호는 Finder·탐색기에서 입력합니다",
        ));
    }
    let mut parts = Vec::new();
    for part in path.split('/').filter(|p| !p.is_empty()) {
        let mut decoded = Vec::new();
        let mut rest = part.as_bytes();
        while let Some((&b, tail)) = rest.split_first() {
            if url && b == b'%' {
                let n = rest
                    .get(1..3)
                    .and_then(|b| std::str::from_utf8(b).ok())
                    .and_then(|h| u8::from_str_radix(h, 16).ok())
                    .ok_or_else(|| error("공유폴더 주소의 % 표기를 확인하세요"))?;
                decoded.push(n);
                rest = &rest[3..];
            } else {
                decoded.push(b);
                rest = tail;
            }
        }
        let text =
            String::from_utf8(decoded).map_err(|_| error("공유폴더 이름을 UTF-8로 입력하세요"))?;
        name(&text)?;
        if text
            .chars()
            .any(|c| matches!(c, ':' | '*' | '?' | '"' | '<' | '>' | '|'))
        {
            return Err(error("공유폴더 이름을 확인하세요"));
        }
        parts.push(text);
    }
    if windows {
        let server = ipv6.map_or_else(
            || server.to_string(),
            |ip| format!("{}.ipv6-literal.net", ip.to_string().replace(':', "-")),
        );
        Ok(format!(
            "\\\\{server}{}",
            if parts.is_empty() {
                String::new()
            } else {
                format!("\\{}", parts.join("\\"))
            }
        ))
    } else {
        let mut out = format!("smb://{server}");
        for part in parts {
            out.push('/');
            for b in part.bytes() {
                if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'~') {
                    out.push(b as char);
                } else {
                    use std::fmt::Write;
                    let _ = write!(out, "%{b:02X}");
                }
            }
        }
        Ok(out)
    }
}
pub fn open_native(address: &str) -> io::Result<()> {
    let target = smb_target(address, cfg!(windows))?;
    #[cfg(windows)]
    {
        std::process::Command::new(
            std::env::var_os("SystemRoot")
                .map(PathBuf::from)
                .unwrap_or_else(|| "C:\\Windows".into())
                .join("explorer.exe"),
        )
        .arg(target)
        .spawn()?;
    }
    #[cfg(not(windows))]
    {
        std::process::Command::new("/usr/bin/open")
            .args(["-a", "Finder", &target])
            .spawn()?;
    }
    Ok(())
}
fn local_folder(p: &str) -> io::Result<String> {
    if p.is_empty() || p.chars().any(|c| c.is_control()) {
        return Err(error("폴더 경로가 올바르지 않습니다"));
    }
    let p = canonical(p)?;
    if !Path::new(&p).is_dir() {
        return Err(error("폴더를 선택하세요"));
    }
    Ok(p)
}
/// Opening a local item is an explicit user action; Explorer always receives a path as a
/// single argument rather than interpreting a remote item as a program.
fn open_local(p: &str) -> io::Result<()> {
    if p.chars().any(|c| c.is_control()) {
        return Err(error("파일 경로가 올바르지 않습니다"));
    }
    #[cfg(target_os = "macos")]
    {
        std::process::Command::new("/usr/bin/open")
            .arg("--")
            .arg(p)
            .spawn()?;
    }
    #[cfg(windows)]
    {
        let p = crate::ui::wide(p);
        let result = unsafe {
            windows_sys::Win32::UI::Shell::ShellExecuteW(
                std::ptr::null_mut(),
                crate::ui::wide("open").as_ptr(),
                p.as_ptr(),
                std::ptr::null(),
                std::ptr::null(),
                windows_sys::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL,
            )
        };
        if (result as isize) <= 32 {
            return Err(error("기본 앱에서 파일을 열 수 없습니다"));
        }
    }
    Ok(())
}

/// Atomic no-clobber rename: a target created by another job between stat and rename must
/// not be overwritten unless the user explicitly chose replacement.
fn rename_local(from: &str, to: &str, replace: bool) -> io::Result<()> {
    if from.contains('\0') || to.contains('\0') {
        return Err(error("파일 경로에 NUL 문자를 넣을 수 없습니다"));
    }
    #[cfg(target_os = "macos")]
    {
        unsafe extern "C" {
            fn renamex_np(
                from: *const std::ffi::c_char,
                to: *const std::ffi::c_char,
                flags: u32,
            ) -> std::ffi::c_int;
        }
        let from = std::ffi::CString::new(from).map_err(|_| error("잘못된 원본 경로"))?;
        let to = std::ffi::CString::new(to).map_err(|_| error("잘못된 대상 경로"))?;
        if unsafe { renamex_np(from.as_ptr(), to.as_ptr(), if replace { 0 } else { 4 }) } != 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(())
    }
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt;
        let a: Vec<u16> = std::ffi::OsStr::new(from)
            .encode_wide()
            .chain(Some(0))
            .collect();
        let b: Vec<u16> = std::ffi::OsStr::new(to)
            .encode_wide()
            .chain(Some(0))
            .collect();
        let flags = if replace {
            windows_sys::Win32::Storage::FileSystem::MOVEFILE_REPLACE_EXISTING
        } else {
            0
        };
        if unsafe {
            windows_sys::Win32::Storage::FileSystem::MoveFileExW(a.as_ptr(), b.as_ptr(), flags)
        } == 0
        {
            return Err(io::Error::last_os_error());
        }
        Ok(())
    }
    #[cfg(not(any(windows, target_os = "macos")))]
    {
        if !replace && Path::new(to).try_exists()? {
            return Err(io::Error::new(
                io::ErrorKind::AlreadyExists,
                "같은 이름의 파일이 있습니다",
            ));
        }
        std::fs::rename(from, to)
    }
}

pub fn shutdown() {
    let ids: Vec<_> = connections().lock().unwrap().keys().cloned().collect();
    for id in ids {
        disconnect(&id);
    }
    for c in connecting().lock().unwrap().values() {
        c.stop();
    }
    for (_, c) in jobs().lock().unwrap().values() {
        c.stop();
    }
}

#[cfg(test)]
mod tests {
    /// This PC: the system drive is listed as a folder named like Explorer's drive letter.
    #[cfg(windows)]
    #[test]
    fn drives_list_the_system_drive() {
        let d = drives();
        assert!(d.iter().any(|e| e.name == "C:" && e.dir), "{:?}", d.iter().map(|e| &e.name).collect::<Vec<_>>());
    }

    use super::*;

    #[test]
    fn reveal_accepts_real_folders_not_files_or_virtual_roots() {
        let root = std::env::temp_dir().join(format!("itda-reveal-{}", crate::store::new_id()));
        let package = root.join("Fixture.app");
        std::fs::create_dir_all(&package).unwrap();
        let file = root.join("must-not-open.exe");
        let missing = root.join("missing");
        std::fs::write(&file, b"generated test file").unwrap();
        assert!(Path::new(&local_folder(package.to_str().unwrap()).unwrap()).ends_with("Fixture.app"));
        for invalid in ["", "\n", file.to_str().unwrap(), missing.to_str().unwrap()] {
            assert!(local_folder(invalid).is_err(), "accepted {invalid:?}");
        }
        assert_eq!(std::fs::read(&file).unwrap(), b"generated test file");
        std::fs::remove_dir_all(root).unwrap();
    }

    /// A quit that closed admissions sees every transfer already in, and refuses later ones.
    #[test]
    fn closed_admissions_refuse_new_transfers() {
        let control = Arc::new(Control::default());
        let job = crate::store::new_id();
        let before = close_admissions();
        assert!(!admit(&job, "admit-test", &control), "a transfer started while Itda was closing");
        assert!(!jobs().lock().unwrap().contains_key(&job));
        let _ = before;
        open_admissions();
        assert!(admit(&job, "admit-test", &control));
        jobs().lock().unwrap().remove(&job);
        // A refused transfer is reported as failed, with nothing started.
        let (tx, rx) = std::sync::mpsc::channel();
        let tx = Mutex::new(tx);
        close_admissions();
        transfer::start(json!({"from":"local","to":"local","source":"/a","destination":"/b","names":["x"],"policy":"keep","req":"r"}), Host::default(), Arc::new(move |e: Value| { let _ = tx.lock().unwrap().send(e); }));
        open_admissions();
        let e = rx.recv_timeout(std::time::Duration::from_secs(5)).unwrap();
        assert!(e["state"] == "failed" && e["error"] == CLOSING, "{e}");
    }

    #[test]
    fn closing_a_page_keeps_its_running_transfer_until_completion() {
        let page = crate::store::new_id();
        let key = scoped_id("release-test", &json!(page));
        let job = crate::store::new_id();
        let control = Arc::new(Control::default());
        jobs().lock().unwrap().insert(job.clone(), (key.clone(), control.clone()));
        release_page(&page);
        assert!(control.check().is_ok(), "closing a page cancelled its transfer");
        assert!(retired().lock().unwrap().contains(&key));
        jobs().lock().unwrap().remove(&job);
        cleanup_retired(&key);
        assert!(!retired().lock().unwrap().contains(&key), "finished page resources were retained");
    }

    #[test]
    fn cancellation_is_owned_by_the_requesting_file_page() {
        let host = Host { id: crate::store::new_id(), ..Default::default() };
        let key = scoped_id(&host.id, &json!("page-a"));
        let job = crate::store::new_id();
        let control = Arc::new(Control::default());
        jobs().lock().unwrap().insert(job.clone(), (key, control.clone()));
        let emit: Emit = Arc::new(|_| {});
        dispatch(json!({"op":"cancel","job":job,"window":"page-b"}), host.clone(), Login::default(), emit.clone());
        assert!(control.check().is_ok(), "another page cancelled this page's job");
        dispatch(json!({"op":"cancel","job":job,"window":"page-a"}), host, Login::default(), emit);
        assert!(control.check().is_err(), "the owning page could not cancel its job");
        jobs().lock().unwrap().remove(&job);
    }

    #[test]
    fn verbatim_prefix_dropped() {
        assert_eq!(plain(r"\\?\C:\Users\me\Docs"), r"C:\Users\me\Docs");
        assert_eq!(plain(r"\\?\UNC\server\share\x"), r"\\server\share\x");
        assert_eq!(plain("/Users/me"), "/Users/me");
        let here = canonical(".").unwrap();
        assert!(!here.starts_with(r"\\?\") && Path::new(&here).is_absolute());
    }

    #[test]
    fn names_windows_cannot_store() {
        for bad in ["C:x.txt", "a:b", "what?", "x*", "a<b", "a>b", "a|b", "q\"", "dot.", "space ", "CON", "con.txt", "NUL", "Com1.log", "LPT9", "CONIN$"] {
            assert!(!windows_name(bad), "{bad}");
        }
        for bad in ["COM¹", "com².pdf", "COM³.txt", "LPT¹", "lpt².log", "LPT³.doc"] {
            assert!(!windows_name(bad), "{bad}");
        }
        for ok in ["report.txt", "CONTENT.txt", "COM10", "console", ".bashrc", "a b.txt", "한글 이름.pdf"] {
            assert!(windows_name(ok), "{ok}");
        }
        assert_eq!(name("C:x.txt").is_ok(), !cfg!(windows));
    }

    /// One unusable name (macOS's "Icon\r", a name that isn't UTF-8) leaves the rest listed.
    #[test]
    fn odd_local_names_left_out() {
        let root = std::env::temp_dir().join(format!("itda-list-{}", crate::store::new_id()));
        std::fs::create_dir(&root).unwrap();
        std::fs::write(root.join("ok.txt"), b"x").unwrap();
        #[cfg(unix)]
        {
            std::fs::write(root.join("Icon\r"), b"").unwrap();
            use std::os::unix::ffi::OsStrExt;
            let _ = std::fs::write(root.join(std::ffi::OsStr::from_bytes(b"bad\xff")), b"");
        }
        let names: Vec<String> = local_list(root.to_str().unwrap()).unwrap().into_iter().map(|e| e.name).collect();
        assert_eq!(names, ["ok.txt"]);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn only_known_safe_types_open_remotely() {
        for bad in ["run.pyw", "x.hta", "x.terminal", "a.exe", "a.scr", "a.command", "a.py", "a.sh", "a.app", "page.html", "img.svg", "noext", ".bashrc", "a.pdf.exe"] {
            assert!(!previewable(bad), "{bad}");
        }
        for ok in ["notes.txt", "Report.PDF", "photo.jpg", "shot.png", "a b/c.heic", "/remote/한글.docx", "clip.mov", "song.mp3"] {
            assert!(previewable(ok), "{ok}");
        }
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn previews_marked_downloaded() {
        let p = std::env::temp_dir().join(format!("itda-q-{}.pdf", crate::store::new_id()));
        std::fs::write(&p, b"%PDF").unwrap();
        mark_downloaded(&p);
        let out = std::process::Command::new("/usr/bin/xattr").args(["-p", "com.apple.quarantine"]).arg(&p).output().unwrap();
        std::fs::remove_file(&p).unwrap();
        assert!(String::from_utf8_lossy(&out.stdout).contains(";Itda;"), "{out:?}");
    }

    #[test]
    fn native_file_targets() {
        assert_eq!(smb_target("smb://nas/Docs", true).unwrap(), r"\\nas\Docs");
        assert_eq!(
            smb_target(r"\\nas\Shared Docs", false).unwrap(),
            "smb://nas/Shared%20Docs"
        );
        for p in [
            "https://nas/x",
            "smb://user:pass@nas/x",
            "smb://nas/../x",
            "smb://nas/hi\n",
        ] {
            assert!(smb_target(p, true).is_err());
        }
    }
    #[test]
    fn saved_account_identity_and_custom_user() {
        let mut host = Host {
            host: "example.local:5900".into(),
            ..Default::default()
        };
        let login = Login {
            user: "saved".into(),
            ssh_key: "saved-key".into(),
            key_pass: Some("key-pass".into()),
            ..Default::default()
        };
        let inherited = cfg(&host, &login, Some("typed-pass".into())).unwrap();
        assert_eq!(inherited.user, "saved");
        assert_eq!(inherited.key, "saved-key");
        assert_eq!(inherited.host, "example.local");
        host.ssh.port = 2222;
        assert_eq!(
            cfg(&host, &login, Some("typed-pass".into())).unwrap().port,
            2222
        );
        host.files.user = "another-user".into();
        let custom = cfg(&host, &login, Some("typed-pass".into())).unwrap();
        assert!(custom.key.is_empty());
        assert!(custom.key_pass.is_empty());
        let defaults: Options = serde_json::from_str("{}").unwrap();
        assert_eq!(defaults.method, "auto");
        assert!(defaults.account.is_empty());
    }
    /// A Windows remote (told by the device or by its "/C:/..." home) gets Windows' name
    /// rules whatever this device is, and the error names the item.
    #[test]
    fn windows_remote_names_and_paths() {
        for root in ["/C:/Users/me", "C:/Users/me", "/d:/"] {
            assert!(windows_root(root), "{root}");
        }
        for root in ["/home/me", "/Users/me", "/", "/C"] {
            assert!(!windows_root(root), "{root}");
        }
        let e = windows_target("CON", "Folder/CON").unwrap_err().to_string();
        assert!(e.contains("Folder/CON"), "{e}");
        assert!(windows_target("trail.", "trail.").is_err());
        assert!(windows_target("한글 it's a file.txt", "x").is_ok());
        assert_eq!(remote_path(r"\\?\C:\Users\me", "/C:/Users/me"), r"C:\Users\me");
        assert_eq!(remote_path(r"\\?\UNC\nas\share", "/"), r"\\nas\share");
        assert_eq!(remote_path("~", "/C:/Users/me"), "/C:/Users/me");
        assert_eq!(remote_path("~/Docs", "/home/me/"), "/home/me/Docs");
        assert_eq!(remote_path("/C:/Users", "/"), "/C:/Users");
    }
    #[test]
    fn file_names() {
        for n in ["", ".", "..", "a/b", "a\\b", "bad\0"] {
            assert!(name(n).is_err());
        }
        assert!(name("한글 '문서'.txt").is_ok());
    }
}

#[cfg(test)]
mod preview_slot_tests {
    use super::*;

    /// A finished preview's session is the next one's; a stopped one's never is, even when it
    /// finishes successfully after the stop; an older request arriving late doesn't run and
    /// doesn't stop the newer one; disconnecting stops the working one and drops the idle one.
    #[test]
    fn preview_slot_lifecycle() {
        let slot = PreviewSlot::<u32>::default();
        let (a, idle) = slot.begin(1).unwrap();
        assert!(idle.is_none());
        slot.finish(&a, Some(7));
        let (b, idle) = slot.begin(2).unwrap();
        assert_eq!(idle, Some(7), "kept session reused");
        // c starts while b is working: b is stopped; b's late success isn't kept.
        let (c, idle) = slot.begin(3).unwrap();
        assert!(idle.is_none() && b.check().is_err());
        slot.finish(&b, Some(8));
        assert!(slot.state.lock().unwrap().0.is_none(), "stopped preview's session kept");
        // An older request (2) arriving after 3 started doesn't run and doesn't stop 3.
        assert!(slot.begin(2).is_none());
        assert!(c.check().is_ok());
        slot.finish(&c, Some(9));
        // A too-large or refused file still hands its session on (finish with Some).
        let (d, idle) = slot.begin(4).unwrap();
        assert_eq!(idle, Some(9));
        slot.finish(&d, None); // a failed session isn't offered
        assert!(slot.state.lock().unwrap().0.is_none());
        let (e, _) = slot.begin(5).unwrap();
        slot.finish(&e, Some(10));
        let (f, _) = slot.begin(6).unwrap();
        slot.state.lock().unwrap().0 = Some(11);
        slot.stop();
        assert!(f.check().is_err() && slot.state.lock().unwrap().0.is_none());
    }
}

fn window_page(v: &Value) -> &str { v["window"].as_str().unwrap_or("") }
fn preview_media(ticket: Option<&preview::Ticket>, bytes: Vec<u8>, mime: &str, first_page: bool) -> io::Result<Value> {
    if let Some(ticket) = ticket {
        Ok(json!({"mime":mime,"url":preview::publish(ticket, bytes, mime)?,"first_page":first_page}))
    } else {
        // The HTTP fixture bridge has no native window/protocol handler.
        Ok(json!({"mime":mime,"base64":crate::ws::base64(&bytes),"first_page":first_page}))
    }
}

fn preview_prepared(ticket: Option<&preview::Ticket>, content: preview_cache::Prepared) -> io::Result<Value> {
    preview_media(ticket, content.bytes.as_ref().clone(), &content.mime, content.first_page)
}

#[cfg(test)]
mod preview_pipeline_tests {
    use super::*;
    use image::{DynamicImage, ImageFormat, ImageReader};
    use std::{io::Cursor, sync::mpsc, time::Duration};
    #[test]
    fn newer_remote_preview_cancels_before_waiting_for_memory_budget() {
        let held: Vec<_> = (0..4).map(|_| preview_image::work(None).unwrap()).collect();
        let slot=Arc::new(PreviewSlot::<u32>::default());
        let (old,_) = slot.begin(1).unwrap();
        let (tx,rx)=mpsc::channel(); let pending=slot.clone();
        let worker=std::thread::spawn(move || { let _=tx.send(preview_remote_begin(&pending,2,None)); });
        let by=std::time::Instant::now()+Duration::from_secs(1);
        while old.check().is_ok() && std::time::Instant::now()<by { std::thread::sleep(Duration::from_millis(10)); }
        let cancelled_before_capacity=old.check().is_err();
        drop(held);
        let (control,kept,permit)=rx.recv_timeout(Duration::from_secs(3)).unwrap().unwrap();
        slot.finish(&control,kept); drop(permit); worker.join().unwrap();
        assert!(cancelled_before_capacity,"new request waited for capacity before stopping the old download");
    }
    #[test]
    fn local_preview_pipeline_refreshes_metadata_and_never_serves_a_removed_file() {
        let owner = format!("preview-test-{}", crate::store::new_id());
        let page = format!("{owner}-page");
        let host = Host {id:owner.clone(),..Default::default()};
        let root = std::env::temp_dir().join(&owner); std::fs::create_dir(&root).unwrap();
        struct Cleanup(PathBuf,String);
        impl Drop for Cleanup { fn drop(&mut self) { preview::revoke_owner(&self.1); preview_cache::invalidate(&self.1); let _=std::fs::remove_dir_all(&self.0); } }
        let _cleanup=Cleanup(root.clone(),owner.clone());
        let path=root.join("fixture.png");
        DynamicImage::new_rgba8(2048,1024).save_with_format(&path,ImageFormat::Png).unwrap();
        preview::bind_page(&page,&owner);
        let ask=|seq| {
            let (tx,rx)=mpsc::channel();
            dispatch(json!({"op":"preview","window":page,"path":path,"side":"local","seq":seq}),host.clone(),Login::default(),Arc::new(move |m| { let _=tx.send(m); }));
            rx.recv_timeout(Duration::from_secs(10)).unwrap()
        };
        let response=|m:&Value| preview::response(&owner,&wry::http::Request::builder().uri(m["data"]["url"].as_str().unwrap()).body(Vec::new()).unwrap());
        let first=ask(1); assert_eq!(first["ok"],true,"{first}"); assert!(first["data"].get("base64").is_none());
        let bytes=response(&first).into_body().into_owned();
        assert_eq!(ImageReader::with_format(Cursor::new(&bytes),ImageFormat::Png).into_dimensions().unwrap(),(1600,800));
        let second=ask(2); assert_eq!(second["ok"],true,"{second}");
        assert_ne!(first["data"]["url"],second["data"]["url"]); assert_eq!(response(&second).body().as_ref(),bytes);
        DynamicImage::new_rgba8(8,4).save_with_format(&path,ImageFormat::Png).unwrap();
        let changed=ask(3); assert_eq!(changed["ok"],true,"{changed}");
        assert_eq!(response(&changed).body().as_ref(),std::fs::read(&path).unwrap());
        // A full image GET is now consumed after delivery. Keep a fresh capability unconsumed
        // (HEAD only) to prove failed metadata refresh revokes it, rather than relying on GET.
        let waiting=ask(4); assert_eq!(waiting["ok"],true);
        let head=|m:&Value| preview::response(&owner,&wry::http::Request::builder().method("HEAD").uri(m["data"]["url"].as_str().unwrap()).body(Vec::new()).unwrap());
        assert_eq!(head(&waiting).status(),200);
        std::fs::remove_file(&path).unwrap(); let removed=ask(5); assert_eq!(removed["ok"],false);
        assert_eq!(head(&waiting).status(),404);
    }
}

/// Cancellation must precede waiting for bounded preview memory: an older download may own
/// the very permit the newer request needs. Return every taken idle session on admission error.
fn preview_remote_begin<T>(slot: &PreviewSlot<T>, generation: u64, ticket: Option<&preview::Ticket>) -> io::Result<(Arc<Control>, Option<T>, preview_image::Work)> {
    let Some((control, kept)) = slot.begin(generation) else {
        return Err(io::Error::new(io::ErrorKind::Interrupted, "더 최근 미리보기가 있습니다"));
    };
    match preview_image::work(ticket) {
        Ok(work) => Ok((control, kept, work)),
        Err(e) => { slot.finish(&control, kept); Err(e) }
    }
}

fn mutates_files(op: &str) -> bool { matches!(op,"undo"|"mkdir"|"rename"|"trash") }

#[cfg(any(windows,test))]
pub(crate) fn preview_panic_is_handled() -> bool { preview_image::panic_is_handled() }
