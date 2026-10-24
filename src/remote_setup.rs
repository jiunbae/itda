//! User-selected configuration provisioning. Secrets travel only over SSH stdin and
//! are imported through the destination's native credential store.
use crate::store::{self, Account, Host, Settings};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    collections::{HashMap, HashSet},
    fs,
    io::{self, Read, Write},
    path::{Path, PathBuf},
    process::Stdio,
};
use zeroize::{Zeroize, Zeroizing};

const MAX_PACKAGE: u64 = 16 * 1024 * 1024;
const MAX_KEY: u64 = 512 * 1024;
const SCHEMA: u32 = 1;

#[derive(Clone, Default, Deserialize)]
#[serde(default)]
pub struct Selection {
    pub host_ids: Vec<String>,
    pub settings: bool,
    pub passwords: bool,
    pub private_keys: bool,
    pub close_running: bool,
    pub launch: bool,
}

#[derive(Clone, Serialize, Deserialize)]
struct Preferences {
    theme: String,
    tray: bool,
    thumbnails: bool,
    notify: bool,
    hotkey: String,
    toolbar: String,
    mac_scroll: String,
    scroll_accel: bool,
    view: String,
    zoom: u16,
    defaults: store::HostOpts,
    auto_update: bool,
}
impl Preferences {
    fn from(s: &Settings) -> Self {
        Self {
            theme: s.theme.clone(),
            tray: s.tray,
            thumbnails: s.thumbnails,
            notify: s.notify,
            hotkey: s.hotkey.clone(),
            toolbar: s.toolbar.clone(),
            mac_scroll: s.mac_scroll.clone(),
            scroll_accel: s.scroll_accel,
            view: s.view.clone(),
            zoom: s.zoom,
            defaults: s.defaults.clone(),
            auto_update: s.auto_update,
        }
    }
    fn apply(&self, s: &mut Settings) {
        s.theme = self.theme.clone();
        s.tray = self.tray;
        s.thumbnails = self.thumbnails;
        s.notify = self.notify;
        s.hotkey = self.hotkey.clone();
        s.toolbar = self.toolbar.clone();
        s.mac_scroll = self.mac_scroll.clone();
        s.scroll_accel = self.scroll_accel;
        s.view = self.view.clone();
        s.zoom = self.zoom.clamp(50, 200);
        s.defaults = self.defaults.clone();
        s.auto_update = self.auto_update;
        s.onboarded = true;
    }
}

#[derive(Serialize, Deserialize)]
enum SecretKind {
    Account,
    Host,
    Files,
    Hp,
    KeyPass,
}
#[derive(Serialize, Deserialize)]
struct Secret {
    kind: SecretKind,
    owner: String,
    value: String,
}
impl Drop for Secret {
    fn drop(&mut self) {
        self.value.zeroize();
    }
}
#[derive(Serialize, Deserialize)]
struct Key {
    account: String,
    bytes: Vec<u8>,
}
impl Drop for Key {
    fn drop(&mut self) {
        self.bytes.zeroize();
    }
}
#[derive(Serialize, Deserialize)]
pub struct Package {
    schema: u32,
    origin: String,
    hosts: Vec<Host>,
    accounts: Vec<Account>,
    preferences: Option<Preferences>,
    secrets: Vec<Secret>,
    keys: Vec<Key>,
}

// Confirmation metadata comes from the immutable package behind the plan token.
// Never serialize the package itself: it contains passwords and private-key bytes.
fn package_identities(package: &Package) -> Value {
    let account_name = |id: &str| package.accounts.iter().find(|a| a.id == id)
        .map(|a| if a.name.is_empty() { a.user.as_str() } else { a.name.as_str() }).unwrap_or("");
    json!({
        "hosts": package.hosts.iter().map(|h| json!({
            "id": h.id, "name": if h.name.is_empty() { &h.host } else { &h.name },
            "address": h.host, "user": h.user, "account": account_name(&h.account),
            "filesAddress": h.files.host, "filesUser": h.files.user,
            "filesAccount": account_name(&h.files.account)
        })).collect::<Vec<_>>(),
        "accounts": package.accounts.iter().map(|a| json!({
            "id": a.id, "name": if a.name.is_empty() { &a.user } else { &a.name }, "user": a.user,
            "key": package.keys.iter().any(|k| k.account == a.id),
            "password": package.secrets.iter().any(|secret| matches!(secret.kind, SecretKind::Account) && secret.owner == a.id)
        })).collect::<Vec<_>>()
    })
}

fn error(s: &str) -> io::Error {
    io::Error::other(s)
}
fn mapped(origin: &str, kind: &str, id: &str) -> String {
    let hash = Sha256::digest(format!("itda-config-v1\0{origin}\0{kind}\0{id}"));
    hash[..16].iter().map(|v| format!("{v:02x}")).collect()
}
fn origin(root: &Path) -> io::Result<String> {
    let path = root.join("sharing-origin");
    match fs::read_to_string(&path) {
        Ok(s) if !s.trim().is_empty() => Ok(s.trim().into()),
        Ok(_) => Err(error("설정 공유 식별자가 올바르지 않습니다")),
        Err(e) if e.kind() == io::ErrorKind::NotFound => {
            let value = random_id()?;
            write_private_new(&path, value.as_bytes())?;
            Ok(value)
        }
        Err(e) => Err(e),
    }
}
fn source_key(path: &str) -> io::Result<Vec<u8>> {
    let path = path
        .strip_prefix("~/")
        .map(|p| {
            std::env::var_os("HOME")
                .or_else(|| std::env::var_os("USERPROFILE"))
                .map(PathBuf::from)
                .unwrap_or_default()
                .join(p)
        })
        .unwrap_or_else(|| PathBuf::from(path));
    let file = fs::File::open(path)
        .map_err(|_| error("선택한 계정의 SSH 개인키 파일을 읽지 못했습니다"))?;
    if !file.metadata()?.is_file() {
        return Err(error("SSH 개인키는 일반 파일이어야 합니다"));
    }
    let mut bytes = Vec::new();
    file.take(MAX_KEY + 1).read_to_end(&mut bytes)?;
    if bytes.is_empty() || bytes.len() as u64 > MAX_KEY {
        bytes.zeroize();
        return Err(error("SSH 개인키 파일 크기가 올바르지 않습니다"));
    }
    Ok(bytes)
}

fn validate_account_bindings(hosts: &[Host], account_ids: &HashSet<&str>) -> io::Result<()> {
    for host in hosts {
        let name = if !host.name.is_empty() {
            host.name.as_str()
        } else if !host.host.is_empty() {
            host.host.as_str()
        } else {
            "이름 없는 연결"
        };
        if !host.account.is_empty() && !account_ids.contains(host.account.as_str()) {
            return Err(error(&format!(
                "‘{name}’의 연결 계정을 찾을 수 없습니다. 연결을 편집해 계정을 다시 선택한 뒤 공유해 주세요."
            )));
        }
        // Empty inherits the device login; custom uses the entered files.user.
        if !host.files.account.is_empty()
            && host.files.account != "custom"
            && !account_ids.contains(host.files.account.as_str())
        {
            return Err(error(&format!(
                "‘{name}’의 파일 연결 계정을 찾을 수 없습니다. 파일 열기에서 계정을 다시 선택한 뒤 공유해 주세요."
            )));
        }
    }
    Ok(())
}

pub fn snapshot(
    hosts: &[Host],
    accounts: &[Account],
    settings: &Settings,
    choice: &Selection,
) -> io::Result<Package> {
    snapshot_with_origin(hosts, accounts, settings, choice, || origin(&store::dir()))
}

/// Pure selection/dependency validation, also used before any SSH probe.
fn selected_profiles(
    hosts: &[Host],
    accounts: &[Account],
    choice: &Selection,
) -> io::Result<(Vec<Host>, Vec<Account>)> {
    let selected: HashSet<_> = choice.host_ids.iter().collect();
    if selected.len() != choice.host_ids.len()
        || selected
            .iter()
            .any(|id| !hosts.iter().any(|h| &h.id == *id))
    {
        return Err(error("공유할 연결을 다시 선택해 주세요"));
    }
    let selected_hosts: Vec<_> = hosts
        .iter()
        .filter(|h| selected.contains(&h.id))
        .cloned()
        .collect();
    let references: HashSet<_> = selected_hosts
        .iter()
        .flat_map(|h| [h.account.as_str(), h.files.account.as_str()])
        .collect();
    let selected_accounts: Vec<_> = accounts
        .iter()
        .filter(|a| references.contains(a.id.as_str()))
        .cloned()
        .collect();
    // Deleted saved accounts can leave a file binding at rest. Refuse that
    // selected source before touching secrets, key files or the origin file.
    validate_account_bindings(
        &selected_hosts,
        &selected_accounts.iter().map(|a| a.id.as_str()).collect(),
    )?;
    Ok((selected_hosts, selected_accounts))
}

fn snapshot_with_origin(
    hosts: &[Host],
    accounts: &[Account],
    settings: &Settings,
    choice: &Selection,
    source_origin: impl FnOnce() -> io::Result<String>,
) -> io::Result<Package> {
    let (mut selected_hosts, mut selected_accounts) = selected_profiles(hosts, accounts, choice)?;
    let mut secrets = Vec::new();
    let mut keys = Vec::new();
    let mut add = |kind, owner: &str, key: &str| -> io::Result<()> {
        if let Some(value) = store::get_password_checked(key)? {
            secrets.push(Secret {
                kind,
                owner: owner.into(),
                value,
            });
        }
        Ok(())
    };
    for h in &mut selected_hosts {
        if choice.passwords {
            add(SecretKind::Host, &h.id, &h.id)?;
            add(SecretKind::Files, &h.id, &crate::files::password_key(h))?;
            add(
                SecretKind::Hp,
                &h.id,
                &crate::hp_upgrade::password_key(&h.id),
            )?;
        }
        // Last pictures and usage history are local, not connection settings.
        h.last_used = 0;
        h.remote_name.clear();
        h.remote_size.clear();
    }
    for a in &mut selected_accounts {
        if choice.passwords {
            add(SecretKind::Account, &a.id, &store::acct_key(&a.id))?;
        }
        if choice.private_keys && !a.ssh_key.is_empty() {
            keys.push(Key {
                account: a.id.clone(),
                bytes: source_key(&a.ssh_key)?,
            });
            if keys.iter().map(|k| k.bytes.len() as u64).sum::<u64>() > MAX_PACKAGE / 4 {
                return Err(error("선택한 SSH 개인키의 전체 크기가 너무 큽니다"));
            }
            add(SecretKind::KeyPass, &a.id, &store::acct_keypass(&a.id))?;
        }
        // A source-machine path must never become a broken destination key path.
        a.ssh_key.clear();
    }
    Ok(Package {
        schema: SCHEMA,
        origin: source_origin()?,
        hosts: selected_hosts,
        accounts: selected_accounts,
        preferences: choice.settings.then(|| Preferences::from(settings)),
        secrets,
        keys,
    })
}

trait Vault {
    fn get(&self, key: &str) -> io::Result<Option<String>>;
    fn set(&mut self, key: &str, value: Option<&str>) -> io::Result<()>;
}
struct NativeVault;
impl Vault for NativeVault {
    fn get(&self, key: &str) -> io::Result<Option<String>> {
        store::get_password_checked(key)
    }
    fn set(&mut self, key: &str, value: Option<&str>) -> io::Result<()> {
        store::set_password_checked(key, value)
    }
}
/// Over SSH a Mac's login keychain is locked to that session (macOS unlocks it per login
/// session), so writing passwords fails. They go instead through a helper the same app runs
/// in the person's desktop session (started with `open`), over a socket in a private folder.
/// Each request carries an id its reply must echo; after any transport or framing failure the
/// channel is poisoned: an outcome that can't be known is never taken as a later answer.
#[cfg(any(target_os = "macos", windows))]
struct SessionVault {
    app: PathBuf,
    link: std::cell::RefCell<Option<Box<dyn Duplex>>>,
    dir: std::cell::RefCell<Option<PathBuf>>,
    next: std::cell::Cell<u64>,
    poisoned: std::cell::Cell<bool>,
}
#[cfg(any(target_os = "macos", windows))]
#[derive(Serialize)]
struct VaultRequest<'a> {
    id: u64,
    op: &'a str,
    key: &'a str,
    value: Option<&'a str>,
}
#[cfg(any(target_os = "macos", windows))]
#[derive(Deserialize)]
struct VaultReply {
    id: u64,
    ok: bool,
    #[serde(default)]
    value: Option<String>,
    #[serde(default)]
    error: Option<String>,
}
/// The helper connection: a Unix socket on a Mac, a loopback TCP connection on Windows.
#[cfg(any(target_os = "macos", windows))]
trait Duplex: Read + Write {}
#[cfg(any(target_os = "macos", windows))]
impl<T: Read + Write> Duplex for T {}
/// One line (without its newline) read byte by byte into wiped memory: no reader buffer keeps
/// a copy of a password.
#[cfg(any(target_os = "macos", windows))]
fn read_line_secret(r: &mut impl Read) -> io::Result<Zeroizing<Vec<u8>>> {
    let mut line = Zeroizing::new(Vec::new());
    let mut b = [0u8; 1];
    loop {
        if r.read(&mut b)? == 0 {
            return Err(io::Error::new(io::ErrorKind::UnexpectedEof, "closed"));
        }
        if b[0] == b'\n' {
            return Ok(line);
        }
        if line.len() > 64 * 1024 {
            return Err(error("키체인 도우미의 응답이 너무 깁니다"));
        }
        line.push(b[0]);
    }
}
#[cfg(any(target_os = "macos", windows))]
impl SessionVault {
    fn new(app: PathBuf) -> SessionVault {
        SessionVault { app, link: Default::default(), dir: Default::default(), next: std::cell::Cell::new(1), poisoned: Default::default() }
    }
    #[cfg(windows)]
    fn start(&self) -> io::Result<()> {
        // A one-time task that runs only in this account's signed-in desktop session
        // (/IT), where Credential Manager is usable; it proves itself with a token from a file
        // in this account's private data folder, then answers on a loopback connection.
        use std::net::TcpListener;
        let dir = store::dir().join(format!("vault-{}", &random_id()?[..16]));
        private_dir(&dir)?;
        *self.dir.borrow_mut() = Some(dir.clone());
        let token = Zeroizing::new(random_id()?);
        fs::write(dir.join("t"), token.as_bytes())?;
        let listener = TcpListener::bind("127.0.0.1:0")?;
        listener.set_nonblocking(true)?;
        let port = listener.local_addr()?.port();
        let task = format!("Itda-{}", dir.file_name().unwrap_or_default().to_string_lossy());
        let run = format!("\"{}\" --vault-helper {port} \"{}\"", self.app.display(), dir.display());
        if run.len() > 261 {
            return Err(error("Itda 설치 경로가 너무 길어 비밀번호 도우미를 시작하지 못했습니다"));
        }
        let schtasks = crate::update::system_tool("schtasks.exe");
        let task_ok = |args: &[&str]| {
            std::process::Command::new(&schtasks)
                .args(args)
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status()
                .is_ok_and(|s| s.success())
        };
        struct Task<'a>(&'a dyn Fn(&[&str]) -> bool, &'a str);
        impl Drop for Task<'_> {
            fn drop(&mut self) {
                (self.0)(&["/Delete", "/F", "/TN", self.1]);
            }
        }
        if !task_ok(&["/Create", "/F", "/TN", &task, "/SC", "ONCE", "/ST", "00:00", "/IT", "/RL", "LIMITED", "/TR", &run]) {
            return Err(error("비밀번호 도우미 작업을 만들지 못했습니다"));
        }
        let _task = Task(&task_ok, &task);
        if !task_ok(&["/Run", "/TN", &task]) {
            return Err(error("비밀번호 도우미를 시작하지 못했습니다. 원격 PC에 이 계정으로 로그인되어 있는지 확인하세요"));
        }
        let until = std::time::Instant::now() + std::time::Duration::from_secs(20);
        let mut stream = loop {
            match listener.accept() {
                Ok((s, _)) => break s,
                Err(e) if e.kind() == io::ErrorKind::WouldBlock && std::time::Instant::now() < until => std::thread::sleep(std::time::Duration::from_millis(100)),
                Err(_) => return Err(error("비밀번호 도우미가 응답하지 않습니다. 원격 PC에 이 계정으로 로그인되어 있는지 확인하세요")),
            }
        };
        stream.set_nonblocking(false)?;
        stream.set_read_timeout(Some(std::time::Duration::from_secs(5)))?;
        let proof = read_line_secret(&mut stream)?;
        if proof.as_slice() != token.as_bytes() {
            return Err(error("비밀번호 도우미를 확인하지 못했습니다"));
        }
        stream.set_read_timeout(Some(std::time::Duration::from_secs(30)))?;
        *self.link.borrow_mut() = Some(Box::new(stream));
        Ok(())
    }
    #[cfg(target_os = "macos")]
    fn start(&self) -> io::Result<()> {
        use std::os::unix::net::UnixListener;
        let dir = store::dir().join(format!("vault-{}", &random_id()?[..16]));
        private_dir(&dir)?;
        *self.dir.borrow_mut() = Some(dir.clone());
        let listener = UnixListener::bind(dir.join("s"))?;
        listener.set_nonblocking(true)?;
        // The helper is told this process's data root (an isolated profile included).
        let base = store::override_base().unwrap_or_default();
        let started = std::process::Command::new("/usr/bin/open")
            .args(["-n", "-g", "-j"])
            .arg(&self.app)
            .args(["--args", "--vault-helper"])
            .arg(&base)
            .arg(dir.join("s"))
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()?;
        if !started.success() {
            return Err(error("이 Mac의 로그인 세션에서 키체인 도우미를 열지 못했습니다. 화면에 로그인한 계정인지 확인하세요"));
        }
        let until = std::time::Instant::now() + std::time::Duration::from_secs(20);
        let stream = loop {
            match listener.accept() {
                Ok((s, _)) => break s,
                Err(e) if e.kind() == io::ErrorKind::WouldBlock && std::time::Instant::now() < until => std::thread::sleep(std::time::Duration::from_millis(100)),
                Err(_) => return Err(error("키체인 도우미가 응답하지 않습니다. 원격 Mac 화면에 로그인되어 있는지 확인하세요")),
            }
        };
        stream.set_nonblocking(false)?;
        stream.set_read_timeout(Some(std::time::Duration::from_secs(30)))?;
        *self.link.borrow_mut() = Some(Box::new(stream));
        Ok(())
    }
    fn call(&self, op: &str, key: &str, value: Option<&str>) -> io::Result<Option<Zeroizing<String>>> {
        if self.poisoned.get() {
            return Err(error("키체인 도우미와의 연결이 끊겨 저장 결과를 확인할 수 없습니다"));
        }
        if self.link.borrow().is_none() {
            if let Err(e) = self.start() {
                self.poisoned.set(true);
                return Err(e);
            }
        }
        let id = self.next.get();
        self.next.set(id + 1);
        let outcome = (|| {
            let mut link = self.link.borrow_mut();
            let stream = link.as_mut().ok_or_else(|| error("closed"))?;
            let mut line = Zeroizing::new(serde_json::to_vec(&VaultRequest { id, op, key, value })?);
            line.push(b'\n');
            stream.write_all(&line)?;
            let reply = read_line_secret(stream)?;
            let reply: VaultReply = serde_json::from_slice(&reply).map_err(|_| error("framing"))?;
            if reply.id != id {
                return Err(error("framing"));
            }
            Ok(reply)
        })();
        match outcome {
            Ok(reply) if reply.ok => Ok(reply.value.map(Zeroizing::new)),
            Ok(reply) => Err(error(reply.error.as_deref().unwrap_or("키체인에 저장하지 못했습니다"))),
            Err(_) => {
                // Unknown outcome: no further request may share or replace this channel.
                self.poisoned.set(true);
                self.link.take();
                Err(error("키체인 도우미와의 연결이 끊겨 저장 결과를 확인할 수 없습니다"))
            }
        }
    }
}
#[cfg(any(target_os = "macos", windows))]
impl Vault for SessionVault {
    fn get(&self, key: &str) -> io::Result<Option<String>> {
        Ok(self.call("get", key, None)?.map(|v| v.as_str().to_string()))
    }
    fn set(&mut self, key: &str, value: Option<&str>) -> io::Result<()> {
        self.call("set", key, value).map(|_| ())
    }
}
#[cfg(any(target_os = "macos", windows))]
impl Drop for SessionVault {
    fn drop(&mut self) {
        self.link.take(); // the helper leaves when the connection closes
        if let Some(dir) = self.dir.take() {
            let _ = fs::remove_dir_all(dir);
        }
    }
}

/// The helper side, started by `open` in the desktop session with the provisioning data root
/// (empty: the default one) and its socket. The socket must sit in a private `vault-` folder
/// directly in that root; then it answers keychain requests until the connection closes.
#[cfg(target_os = "macos")]
fn vault_helper(base: &str, socket: &Path) -> io::Result<()> {
    use std::os::unix::fs::{FileTypeExt, MetadataExt};
    // Before any store access: the isolated profile, if that's what provisioning used.
    if !base.is_empty() {
        std::env::set_var("ITDA_DATA_DIR", base);
    }
    let mine = unsafe { libc_getuid() };
    let unsafe_path = || error("키체인 도우미의 경로가 안전하지 않습니다");
    let dir = socket.parent().ok_or_else(unsafe_path)?;
    let meta = fs::symlink_metadata(dir)?;
    if meta.file_type().is_symlink() || !meta.is_dir() || meta.uid() != mine || meta.mode() & 0o077 != 0 {
        return Err(unsafe_path());
    }
    let root = fs::canonicalize(store::dir())?;
    let real = fs::canonicalize(dir)?;
    if real.parent() != Some(root.as_path()) || !real.file_name().is_some_and(|n| n.to_string_lossy().starts_with("vault-")) {
        return Err(unsafe_path());
    }
    let sock = fs::symlink_metadata(socket)?;
    if !sock.file_type().is_socket() || sock.uid() != mine {
        return Err(unsafe_path());
    }
    let mut stream = std::os::unix::net::UnixStream::connect(socket)?;
    serve_vault(&mut stream, &mut NativeVault)
}

/// The Windows helper, run by the one-time task in the desktop session: the folder must be a
/// `vault-` folder directly in this account's data folder; it sends the token found there.
#[cfg(windows)]
fn vault_helper(port: &str, dir: &Path) -> io::Result<()> {
    let unsafe_path = || error("비밀번호 도우미의 경로가 안전하지 않습니다");
    let port: u16 = port.parse().map_err(|_| unsafe_path())?;
    let root = fs::canonicalize(store::dir())?;
    let real = fs::canonicalize(dir)?;
    if real.parent() != Some(root.as_path()) || !real.file_name().is_some_and(|n| n.to_string_lossy().starts_with("vault-")) {
        return Err(unsafe_path());
    }
    let mut token = Zeroizing::new(fs::read(real.join("t"))?);
    let mut stream = std::net::TcpStream::connect(("127.0.0.1", port))?;
    token.push(b'\n');
    stream.write_all(&token)?;
    serve_vault(&mut stream, &mut NativeVault)
}

/// Answer the provisioning side's requests with `vault` until it closes the connection.
#[cfg(any(target_os = "macos", windows))]
fn serve_vault(stream: &mut (impl Read + Write), vault: &mut impl Vault) -> io::Result<()> {
    #[derive(Deserialize)]
    struct Request {
        id: u64,
        op: String,
        key: String,
        #[serde(default)]
        value: Option<String>,
    }
    loop {
        let line = match read_line_secret(stream) {
            Ok(line) => line,
            Err(e) if e.kind() == io::ErrorKind::UnexpectedEof => return Ok(()),
            Err(e) => return Err(e),
        };
        let request: Request = serde_json::from_slice(&line).map_err(|_| error("알 수 없는 요청"))?;
        let value = request.value.map(Zeroizing::new);
        let result = match request.op.as_str() {
            "get" => vault.get(&request.key).map(|v| v.map(Zeroizing::new)),
            "set" => vault.set(&request.key, value.as_deref().map(String::as_str)).map(|()| None),
            _ => Err(error("알 수 없는 요청")),
        };
        #[derive(Serialize)]
        struct Reply<'a> {
            id: u64,
            ok: bool,
            value: Option<&'a str>,
            error: Option<String>,
        }
        let reply = match &result {
            Ok(v) => Reply { id: request.id, ok: true, value: v.as_deref().map(String::as_str), error: None },
            Err(e) => Reply { id: request.id, ok: false, value: None, error: Some(e.to_string()) },
        };
        let mut out = Zeroizing::new(serde_json::to_vec(&reply)?);
        out.push(b'\n');
        stream.write_all(&out)?;
    }
}
#[cfg(target_os = "macos")]
extern "C" {
    #[link_name = "getuid"]
    fn libc_getuid() -> u32;
}

fn read_json<T: serde::de::DeserializeOwned + Default>(root: &Path, name: &str) -> io::Result<T> {
    match fs::read(root.join(name)) {
        Ok(bytes) => serde_json::from_slice(&bytes)
            .map_err(|_| error("원격 기기의 기존 설정을 읽지 못해 변경하지 않았습니다")),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(T::default()),
        Err(e) => Err(e),
    }
}
fn private_dir(path: &Path) -> io::Result<()> {
    if fs::symlink_metadata(path).is_ok_and(|m| m.file_type().is_symlink()) {
        return Err(error("공유 설정 폴더가 심볼릭 링크라 변경하지 않았습니다"));
    }
    fs::create_dir_all(path)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o700))?;
    }
    #[cfg(windows)]
    {
        use std::{
            os::windows::{fs::MetadataExt, process::CommandExt},
            process::Command,
        };
        if fs::symlink_metadata(path)?.file_attributes() & 0x400 != 0 {
            return Err(error("공유 설정 폴더가 링크라 변경하지 않았습니다"));
        }
        let script = format!(
            r#"$ErrorActionPreference='Stop';$sid=[Security.Principal.WindowsIdentity]::GetCurrent().User;$acl=New-Object Security.AccessControl.DirectorySecurity;$acl.SetOwner($sid);$acl.SetAccessRuleProtection($true,$false);$acl.AddAccessRule((New-Object Security.AccessControl.FileSystemAccessRule($sid,'FullControl','ContainerInherit,ObjectInherit','None','Allow')));$system=New-Object Security.Principal.SecurityIdentifier('S-1-5-18');$acl.AddAccessRule((New-Object Security.AccessControl.FileSystemAccessRule($system,'FullControl','ContainerInherit,ObjectInherit','None','Allow')));[IO.Directory]::SetAccessControl('{}',$acl)"#,
            path.to_string_lossy().replace('\'', "''")
        );
        let command = ps(&script);
        let encoded = command.split_whitespace().last().unwrap();
        let ok = Command::new(crate::update::system_tool("powershell.exe"))
            .args(["-NoProfile", "-NonInteractive", "-EncodedCommand", encoded])
            .creation_flags(0x08000000)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()?
            .success();
        if !ok {
            return Err(error("개인키 폴더의 접근 권한을 설정하지 못했습니다"));
        }
    }
    Ok(())
}
fn write_private_new(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let mut opts = fs::OpenOptions::new();
    opts.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        opts.mode(0o600);
    }
    let mut f = opts.open(path)?;
    let result = f.write_all(bytes).and_then(|_| f.sync_all());
    drop(f);
    if result.is_err() {
        let _ = fs::remove_file(path);
    }
    result
}
pub struct ConfigurationLock(fs::File);
impl Drop for ConfigurationLock {
    fn drop(&mut self) {
        let _ = self.0.unlock();
    }
}
fn lock_at(root: &Path) -> io::Result<ConfigurationLock> {
    fs::create_dir_all(root)?;
    let f = fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(root.join("configuration.lock"))?;
    f.try_lock().map_err(|_| {
        error("이 기기의 Itda가 설정을 사용 중입니다. Itda를 종료한 뒤 다시 시도하세요")
    })?;
    Ok(ConfigurationLock(f))
}
pub fn application_lock() -> io::Result<ConfigurationLock> {
    lock_at(&store::dir())
}

fn validate(p: &Package) -> io::Result<()> {
    if p.schema != SCHEMA
        || p.origin.is_empty()
        || p.origin.len() > 128
        || p.hosts.len() > 10000
        || p.accounts.len() > 10000
        || p.keys.len() > 1000
        || p.secrets.len() > 50000
    {
        return Err(error("공유할 설정의 형식이나 크기가 올바르지 않습니다"));
    }
    for ids in [
        p.hosts.iter().map(|h| h.id.as_str()).collect::<Vec<_>>(),
        p.accounts.iter().map(|a| a.id.as_str()).collect(),
    ] {
        let unique: HashSet<_> = ids.iter().collect();
        if unique.len() != ids.len() || ids.iter().any(|s| s.is_empty()) {
            return Err(error("공유 설정에 중복되거나 빈 식별자가 있습니다"));
        }
    }
    // Secrets must belong to a section actually selected in this package. Check
    // before key/backup/configuration writes or any native credential-store call.
    let account_ids: HashSet<_> = p.accounts.iter().map(|a| a.id.as_str()).collect();
    let host_ids: HashSet<_> = p.hosts.iter().map(|h| h.id.as_str()).collect();
    if p.secrets.iter().any(|secret| !match secret.kind {
        SecretKind::Account | SecretKind::KeyPass => account_ids.contains(secret.owner.as_str()),
        SecretKind::Host | SecretKind::Hp | SecretKind::Files => host_ids.contains(secret.owner.as_str()),
    }) {
        return Err(error("공유할 암호의 계정 또는 연결이 선택되지 않았습니다"));
    }
    validate_account_bindings(&p.hosts, &account_ids)?;
    let key_ids: HashSet<_> = p.keys.iter().map(|k| &k.account).collect();
    if key_ids.len() != p.keys.len()
        || p.keys.iter().map(|k| k.bytes.len() as u64).sum::<u64>() > MAX_PACKAGE / 4
        || p.keys.iter().any(|k| {
            k.bytes.is_empty()
                || k.bytes.len() as u64 > MAX_KEY
                || !p.accounts.iter().any(|a| a.id == k.account)
        })
    {
        return Err(error("공유할 SSH 개인키가 올바르지 않습니다"));
    }
    Ok(())
}
fn apply(root: &Path, p: &Package, vault: &mut dyn Vault) -> io::Result<Value> {
    validate(p)?;
    let _lock = lock_at(root)?;
    apply_locked(root, p, vault)
}
fn apply_locked(root: &Path, p: &Package, vault: &mut dyn Vault) -> io::Result<Value> {
    // Validation and the caller's configuration/installation locks still apply.
    // An app-only selection must not parse/rewrite destination configuration:
    // typed round-trips would discard fields this version does not understand.
    let outcome = || json!({"ok":true,"hosts":p.hosts.len(),"accounts":p.accounts.len(),"secrets":p.secrets.len(),"keys":p.keys.len()});
    if p.hosts.is_empty() && p.accounts.is_empty() && p.preferences.is_none() && p.secrets.is_empty() && p.keys.is_empty() {
        return Ok(outcome());
    }
    let change_hosts = !p.hosts.is_empty();
    let change_accounts = !p.accounts.is_empty();
    let mut hosts: Vec<Host> = if change_hosts { read_json(root, "hosts.json")? } else { Vec::new() };
    let mut accounts: Vec<Account> = if change_accounts { read_json(root, "accounts.json")? } else { Vec::new() };
    let mut settings: Settings = if p.preferences.is_some() { read_json(root, "settings.json")? } else { Settings::default() };
    let account_ids: HashMap<_, _> = p
        .accounts
        .iter()
        .map(|a| (a.id.clone(), mapped(&p.origin, "account", &a.id)))
        .collect();
    let host_ids: HashMap<_, _> = p
        .hosts
        .iter()
        .map(|h| (h.id.clone(), mapped(&p.origin, "host", &h.id)))
        .collect();
    let mut created_keys = Vec::new();
    let mut committed_any = false;
    let result = (|| {
        for a in &p.accounts {
            let mut a = a.clone();
            let old_id = a.id.clone();
            a.id = account_ids[&old_id].clone();
            // An omitted key leaves a destination's existing account key alone.
            a.ssh_key = accounts
                .iter()
                .find(|old| old.id == a.id)
                .map(|old| old.ssh_key.clone())
                .unwrap_or_default();
            if let Some(key) = p.keys.iter().find(|k| k.account == old_id) {
                let folder = root.join("imported-keys");
                private_dir(&folder)?;
                let digest: String = Sha256::digest(&key.bytes)[..12]
                    .iter()
                    .map(|b| format!("{b:02x}"))
                    .collect();
                let path = folder.join(format!("{}-{digest}", a.id));
                match fs::read(&path) {
                    Ok(existing) if existing == key.bytes => {}
                    Ok(_) => return Err(error("기존 SSH 키와 충돌해 설정을 변경하지 않았습니다")),
                    Err(e) if e.kind() == io::ErrorKind::NotFound => {
                        write_private_new(&path, &key.bytes)?;
                        created_keys.push(path.clone());
                    }
                    Err(e) => return Err(e),
                }
                a.ssh_key = path.to_string_lossy().into();
            }
            match accounts.iter_mut().find(|old| old.id == a.id) {
                Some(old) => *old = a,
                None => accounts.push(a),
            }
        }
        for h in &p.hosts {
            let mut h = h.clone();
            h.id = host_ids[&h.id].clone();
            h.account = account_ids.get(&h.account).cloned().unwrap_or_default();
            if let Some(id) = account_ids.get(&h.files.account) {
                h.files.account = id.clone();
            }
            h.normalize_groups();
            match hosts.iter_mut().find(|old| old.id == h.id) {
                Some(old) => *old = h,
                None => hosts.push(h),
            }
        }
        if let Some(prefs) = &p.preferences {
            prefs.apply(&mut settings);
        }
        // Opt-outs are byte-preserving, including unknown fields, permissions,
        // missing files and even invalid JSON. Only selected sections take part
        // in the existing backup/commit/rollback transaction.
        let mut changes = Vec::new();
        if change_accounts { changes.push(("accounts.json", serde_json::to_vec_pretty(&accounts)?)); }
        if change_hosts { changes.push(("hosts.json", serde_json::to_vec_pretty(&hosts)?)); }
        if p.preferences.is_some() { changes.push(("settings.json", serde_json::to_vec_pretty(&settings)?)); }
        let backup = root.join("configuration-backups").join(format!(
            "{}-{}",
            store::now(),
            store::new_id()
        ));
        private_dir(&backup)?;
        let mut original = Vec::new();
        for (name, bytes) in &changes {
            let prior = match fs::read(root.join(name)) {
                Ok(b) => Some(b),
                Err(e) if e.kind() == io::ErrorKind::NotFound => None,
                Err(e) => return Err(e),
            };
            if let Some(prior) = &prior {
                write_private_new(&backup.join(name), prior)?;
            }
            write_private_new(&backup.join(format!("{name}.next")), bytes)?;
            original.push((*name, prior));
        }
        let mut old_secrets: Vec<(String, Option<Zeroizing<String>>)> = Vec::new();
        let transaction = (|| {
            for secret in &p.secrets {
                let key = match secret.kind {
                    SecretKind::Account => store::acct_key(
                        account_ids
                            .get(&secret.owner)
                            .ok_or_else(|| error("암호의 계정이 없습니다"))?,
                    ),
                    SecretKind::KeyPass => store::acct_keypass(
                        account_ids
                            .get(&secret.owner)
                            .ok_or_else(|| error("개인키의 계정이 없습니다"))?,
                    ),
                    SecretKind::Host => host_ids
                        .get(&secret.owner)
                        .ok_or_else(|| error("암호의 연결이 없습니다"))?
                        .clone(),
                    SecretKind::Hp => crate::hp_upgrade::password_key(
                        host_ids
                            .get(&secret.owner)
                            .ok_or_else(|| error("공유 암호의 연결이 없습니다"))?,
                    ),
                    SecretKind::Files => crate::files::password_key(
                        hosts
                            .iter()
                            .find(|h| Some(&h.id) == host_ids.get(&secret.owner))
                            .ok_or_else(|| error("파일 연결의 계정이 없습니다"))?,
                    ),
                };
                old_secrets.push((key.clone(), vault.get(&key)?.map(Zeroizing::new)));
                vault.set(&key, Some(&secret.value))?;
            }
            for (name, _) in &changes {
                fs::rename(backup.join(format!("{name}.next")), root.join(name))?;
                committed_any = true;
            }
            Ok::<_, io::Error>(())
        })();
        if let Err(e) = transaction {
            let mut rollback_ok = true;
            for (key, value) in old_secrets.iter().rev() {
                rollback_ok &= vault.set(key, value.as_ref().map(|v| v.as_str())).is_ok();
            }
            for (name, bytes) in original {
                let path = root.join(name);
                if let Some(bytes) = bytes {
                    rollback_ok &= fs::write(path, bytes).is_ok();
                } else if path.exists() {
                    rollback_ok &= fs::remove_file(path).is_ok();
                }
            }
            if !rollback_ok {
                return Err(error("설정 적용과 복구에 실패했습니다. configuration-backups 폴더의 원본을 보존했습니다"));
            }
            committed_any = false;
            return Err(e);
        }
        Ok(outcome())
    })();
    if result.is_err() && !committed_any {
        for path in created_keys {
            let _ = fs::remove_file(path);
        }
    }
    result
}

fn random_id() -> io::Result<String> {
    let mut bytes = [0u8; 24];
    getrandom::getrandom(&mut bytes)
        .map_err(|_| error("안전한 작업 식별자를 만들지 못했습니다"))?;
    Ok(bytes.iter().map(|b| format!("{b:02x}")).collect())
}

/// Include legacy apps which do not yet participate in configuration.lock.
fn other_app_running() -> io::Result<bool> {
    #[cfg(not(target_os = "windows"))]
    {
        let uid = std::process::Command::new("/usr/bin/id")
            .arg("-u")
            .output()?;
        if !uid.status.success() {
            return Err(error("사용자 확인에 실패했습니다"));
        }
        let out = std::process::Command::new("/bin/ps")
            .args([
                "-U",
                String::from_utf8_lossy(&uid.stdout).trim(),
                "-o",
                "pid=,comm=",
            ])
            .output()?;
        if !out.status.success() {
            return Err(error("실행 중인 앱을 확인하지 못했습니다"));
        }
        Ok(String::from_utf8_lossy(&out.stdout).lines().any(|line| {
            let line = line.trim();
            let Some(i) = line.find(char::is_whitespace) else {
                return false;
            };
            line[..i]
                .parse::<u32>()
                .is_ok_and(|pid| pid != std::process::id())
                && Path::new(line[i..].trim())
                    .file_name()
                    .is_some_and(|name| name == "itda")
        }))
    }
    #[cfg(target_os = "windows")]
    {
        let script = format!(
            r#"$ProgressPreference='SilentlyContinue';$me={};$sid=[Security.Principal.WindowsIdentity]::GetCurrent().User.Value
# Another account's Itda can't be asked for its owner (access denied): it isn't ours, skip it.
# Without WMI access, a process whose path this account may read is its own.
try{{$n=@(Get-CimInstance Win32_Process -Filter "Name='itda.exe'" -ErrorAction Stop|Where-Object{{$_.ProcessId -ne $me -and $(try{{(Invoke-CimMethod -InputObject $_ -MethodName GetOwnerSid -ErrorAction Stop).Sid -eq $sid}}catch{{$false}})}}).Count}}
catch{{$n=@(Get-Process -Name itda -ErrorAction SilentlyContinue|Where-Object{{$_.Id -ne $me -and $_.Path}}).Count}}
[Console]::Write($n)"#,
            std::process::id()
        );
        let bytes: Vec<_> = script.encode_utf16().flat_map(u16::to_le_bytes).collect();
        let out = std::process::Command::new(crate::update::system_tool("powershell.exe"))
            .args([
                "-NoProfile",
                "-NonInteractive",
                "-EncodedCommand",
                &crate::ws::base64(&bytes),
            ])
            .output()?;
        if !out.status.success() {
            return Err(error("실행 중인 앱을 확인하지 못했습니다"));
        }
        let count = String::from_utf8_lossy(&out.stdout)
            .trim()
            .parse::<u32>()
            .map_err(|_| error("실행 중인 앱 응답이 올바르지 않습니다"))?;
        Ok(count > 0)
    }
}

struct StopMarker(PathBuf);
impl Drop for StopMarker {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
    }
}
fn idle_lock(root: &Path, close: bool) -> io::Result<ConfigurationLock> {
    if !other_app_running()? {
        return lock_at(root);
    }
    if !close {
        return Err(error(
            "원격 Itda가 실행 중입니다. 종료 후 적용을 선택하거나 원격 앱을 직접 종료하세요",
        ));
    }
    private_dir(root)?;
    let marker = StopMarker(root.join("remote-setup-stop"));
    fs::write(&marker.0, (store::now() + 30).to_string())?;
    // This is the explicit opt-in close confirmed in the source dialog; it closes sessions/transfers.
    // Older versions without this protocol must be closed directly by the user.
    let until = std::time::Instant::now() + std::time::Duration::from_secs(20);
    loop {
        if !other_app_running()? {
            if let Ok(lock) = lock_at(root) {
                return Ok(lock);
            }
        }
        if std::time::Instant::now() >= until {
            return Err(error("원격 Itda를 안전하게 종료하지 못했습니다. 원격 앱을 직접 종료한 뒤 다시 시도하세요"));
        }
        std::thread::sleep(std::time::Duration::from_millis(250));
    }
}

/// Stage on the destination volume. A failed first rename never removes the original.
fn replace_staged(stage: &Path, target: &Path, backup: &Path) -> io::Result<()> {
    let existed = target.try_exists()?;
    if existed {
        fs::rename(target, backup)?;
    }
    if let Err(e) = fs::rename(stage, target) {
        if existed && fs::rename(backup, target).is_err() {
            return Err(error(&format!(
                "앱 교체와 복구에 실패했습니다. 이전 앱을 {} 에 보존했습니다: {e}",
                backup.display()
            )));
        }
        return Err(e);
    }
    Ok(())
}

fn install_staged() -> io::Result<PathBuf> {
    let source = std::env::current_exe()?;
    #[cfg(not(target_os = "windows"))]
    let (source, target) = {
        let source = source
            .ancestors()
            .nth(3)
            .filter(|p| p.file_name().is_some_and(|n| n == "Itda.app"))
            .ok_or_else(|| error("정식 앱 번들에서 설치를 실행해야 합니다"))?
            .to_path_buf();
        let system = PathBuf::from("/Applications/Itda.app");
        let user = std::env::var_os("HOME")
            .map(PathBuf::from)
            .ok_or_else(|| error("사용자 폴더가 없습니다"))?
            .join("Applications/Itda.app");
        let target = if system.exists() {
            system
        } else if user.exists() {
            user
        } else {
            // Writable check without touching any existing application.
            let test = PathBuf::from("/Applications").join(format!(".itda-write-{}", random_id()?));
            if fs::create_dir(&test).is_ok() {
                let _ = fs::remove_dir(test);
                system
            } else {
                user
            }
        };
        (source, target)
    };
    #[cfg(target_os = "windows")]
    let target = PathBuf::from(
        std::env::var_os("LOCALAPPDATA").ok_or_else(|| error("사용자 앱 폴더가 없습니다"))?,
    )
    .join("Programs/Itda/itda.exe");
    let parent = target.parent().unwrap();
    fs::create_dir_all(parent)?;
    let work = parent.join(format!(".itda-setup-{}", random_id()?));
    fs::create_dir(&work)?;
    #[cfg(not(target_os = "windows"))]
    let stage = work.join("Itda.app");
    #[cfg(target_os = "windows")]
    let stage = work.join("itda.exe");
    let backup = work.join("previous");
    let copy = (|| {
        #[cfg(not(target_os = "windows"))]
        {
            if !std::process::Command::new("/usr/bin/ditto")
                .arg(&source)
                .arg(&stage)
                .status()?
                .success()
            {
                return Err(error("앱을 준비하지 못했습니다"));
            }
        }
        #[cfg(target_os = "windows")]
        {
            fs::copy(&source, &stage)?;
        }
        replace_staged(&stage, &target, &backup)
    })();
    // Preserve recovery material when rollback failed; successful installs can discard it.
    if copy.is_ok() || !backup.exists() {
        let _ = fs::remove_dir_all(&work);
    }
    copy?;
    Ok(target)
}

fn provision(root: &Path, p: &Package, close: bool, launch: bool) -> io::Result<Value> {
    validate(p)?;
    private_dir(root)?;
    let deployment = fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(root.join("provision.lock"))?;
    deployment
        .try_lock()
        .map_err(|_| error("다른 원격 설치가 진행 중입니다. 완료 후 다시 시도하세요"))?;
    let _deployment = ConfigurationLock(deployment);
    let config = idle_lock(root, close)?;
    // No mutation until the package, processes, and both locks have been checked.
    let app = install_staged()?;
    // Over SSH the keychain / Credential Manager is reached through the desktop session (see
    // SessionVault), only when there are passwords to store.
    #[cfg(any(target_os = "macos", windows))]
    let mut result = if std::env::var_os("SSH_CONNECTION").is_some() && !p.secrets.is_empty() {
        apply_locked(root, p, &mut SessionVault::new(app.clone()))
    } else {
        apply_locked(root, p, &mut NativeVault)
    }
    .map_err(|e| {
        error(&format!(
            "앱 설치는 완료됐지만 설정 공유에 실패했습니다: {e}"
        ))
    })?;
    #[cfg(not(any(target_os = "macos", windows)))]
    let mut result = apply_locked(root, p, &mut NativeVault).map_err(|e| {
        error(&format!(
            "앱 설치는 완료됐지만 설정 공유에 실패했습니다: {e}"
        ))
    })?;
    drop(config);
    if launch {
        #[cfg(not(target_os = "windows"))]
        let start = std::process::Command::new("/usr/bin/open")
            .arg(&app)
            .status()
            .map(|s| s.success());
        #[cfg(target_os = "windows")]
        let start = (|| {
            let mut session = 0;
            if unsafe {
                windows_sys::Win32::System::RemoteDesktop::ProcessIdToSessionId(
                    std::process::id(),
                    &mut session,
                )
            } == 0
                || session == 0
            {
                return Err(error("Windows의 SSH 세션에서는 앱 창을 열 수 없습니다"));
            }
            std::process::Command::new(&app)
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
                .map(|_| true)
        })();
        if !matches!(start, Ok(true)) {
            result["launch_error"]=json!("설치와 설정 공유는 완료됐지만 앱을 열지 못했습니다. 원격 기기에서 Itda를 직접 열어 주세요");
        }
    }
    result["version"] = json!(env!("CARGO_PKG_VERSION"));
    Ok(result)
}

/// Headless import before GUI/server/log initialization, including on Windows.
pub fn import_mode() -> bool {
    let mode = std::env::args().nth(1).unwrap_or_default();
    #[cfg(windows)]
    if mode == "--vault-helper" {
        let args: Vec<String> = std::env::args().collect();
        if let Err(e) = vault_helper(args.get(2).map_or("", String::as_str), Path::new(args.get(3).map_or("", String::as_str))) {
            eprintln!("vault helper: {e}");
        }
        return true;
    }
    #[cfg(target_os = "macos")]
    if mode == "--vault-helper" {
        let args: Vec<String> = std::env::args().collect();
        if let Err(e) = vault_helper(args.get(2).map_or("", String::as_str), Path::new(args.get(3).map_or("", String::as_str))) {
            eprintln!("vault helper: {e}");
        }
        return true;
    }
    if mode != "--import-configuration" && mode != "--provision-configuration" {
        return false;
    }
    let result = (|| {
        let mut raw = Zeroizing::new(Vec::new());
        io::stdin().take(MAX_PACKAGE + 1).read_to_end(&mut raw)?;
        if raw.len() as u64 > MAX_PACKAGE {
            return Err(error("공유 설정이 너무 큽니다"));
        }
        let p: Package = serde_json::from_slice(&raw)
            .map_err(|_| error("공유 설정의 형식이 올바르지 않습니다"))?;
        if mode == "--provision-configuration" {
            provision(
                &store::dir(),
                &p,
                std::env::args().any(|a| a == "--close-running"),
                std::env::args().any(|a| a == "--launch"),
            )
        } else {
            apply(&store::dir(), &p, &mut NativeVault)
        }
    })();
    match result {
        Ok(v) => println!("{v}"),
        Err(e) => {
            eprintln!("{}", json!({"ok":false,"error":e.to_string()}));
            std::process::exit(1);
        }
    }
    true
}

#[derive(Clone, Serialize, Deserialize)]
pub struct Remote {
    pub platform: String,
    pub version: String,
    pub running: bool,
}
pub struct Plan {
    pub token: String,
    cfg: crate::tunnel::SshCfg,
    package: Package,
    choice: Selection,
    remote: Remote,
    expires: std::time::Instant,
}
impl Drop for Plan {
    fn drop(&mut self) {
        self.cfg.pass.zeroize();
        self.cfg.key_pass.zeroize();
    }
}
static PLAN: std::sync::Mutex<Option<Plan>> = std::sync::Mutex::new(None);
static BUSY: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
pub struct Busy;
impl Drop for Busy {
    fn drop(&mut self) {
        BUSY.store(false, std::sync::atomic::Ordering::Release);
    }
}
pub fn begin() -> io::Result<Busy> {
    if BUSY.swap(true, std::sync::atomic::Ordering::AcqRel) {
        return Err(error("원격 설치 작업이 이미 진행 중입니다"));
    }
    Ok(Busy)
}
pub fn discard(token: &str) {
    let mut plan = PLAN.lock().unwrap();
    if plan.as_ref().is_some_and(|p| p.token == token) {
        *plan = None;
    }
}
fn q(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}
fn ps(script: &str) -> String {
    let script = format!("[Console]::OutputEncoding=New-Object Text.UTF8Encoding($false);{script}");
    let bytes: Vec<_> = script.encode_utf16().flat_map(u16::to_le_bytes).collect();
    format!(
        "powershell.exe -NoProfile -NonInteractive -ExecutionPolicy Bypass -EncodedCommand {}",
        crate::ws::base64(&bytes)
    )
}
/// A known_hosts record as `ssh-keygen -l` can read it, for display only: comments and revoked
/// keys are dropped, and a CA's marker removed (ssh itself still applies both when connecting).
/// known_hosts allows blanks before a record and after its marker.
fn fingerprint_input(line: &str) -> Option<&str> {
    let line = line.trim_start();
    if line.is_empty() || line.starts_with('#') || line.starts_with("@revoked") {
        return None;
    }
    Some(line.strip_prefix("@cert-authority").map_or(line, str::trim_start))
}

fn trusted_fingerprints(cfg: &crate::tunnel::SshCfg) -> io::Result<Vec<String>> {
    // Resolve SSH aliases locally, before making any authenticated connection.
    let mut resolve = crate::tunnel::ssh_command_strict(cfg)?;
    let output = resolve
        .arg("-G")
        .arg(crate::tunnel::ssh_target(cfg)?)
        .stdin(Stdio::null())
        .output()?;
    if !output.status.success() {
        return Err(error("SSH 설정을 읽지 못했습니다"));
    }
    let text = String::from_utf8_lossy(&output.stdout);
    let field = |name: &str| {
        text.lines()
            .find_map(|line| line.strip_prefix(&format!("{name} ")))
            .map(str::trim)
    };
    let host = field("hostname").ok_or_else(|| error("SSH 기기 이름을 확인하지 못했습니다"))?;
    let port = field("port").unwrap_or("22");
    let lookup = match field("hostkeyalias").filter(|v| *v != "none") {
        Some(alias) => alias.to_owned(),
        None if port == "22" => host.to_owned(),
        None => format!("[{host}]:{port}"),
    };
    let keygen = crate::tunnel::ssh_exe()
        .unwrap()
        .with_file_name(if cfg!(windows) {
            "ssh-keygen.exe"
        } else {
            "ssh-keygen"
        });
    // Every file ssh trusts from (including the one earlier Itda wrote by mistake); a revoked
    // key is never shown as a trusted fingerprint (ssh refuses it anyway).
    let files: Vec<_> = crate::tunnel::known_hosts_files().into_iter().filter(|f| f.is_file()).collect();
    if files.is_empty() {
        return Err(error("처음 SSH로 연결하는 기기입니다. 먼저 Itda의 파일 열기 또는 SSH 화면 연결에서 기기를 확인한 뒤 다시 시도하세요. 원격 설치에서는 새 SSH 호스트 키를 자동 수락하지 않습니다"));
    }
    let mut found = Vec::new();
    for known in &files {
        let keys = std::process::Command::new(&keygen)
            .arg("-F")
            .arg(&lookup)
            .arg("-f")
            .arg(known)
            .stdin(Stdio::null())
            .output()?;
        if keys.status.success() {
            for line in String::from_utf8_lossy(&keys.stdout).lines() {
                if let Some(line) = fingerprint_input(line) {
                    found.extend_from_slice(line.as_bytes());
                    found.push(b'\n');
                }
            }
        }
    }
    if found.is_empty() {
        return Err(error("이 기기의 SSH 호스트 키를 아직 신뢰하지 않았습니다. 먼저 파일 열기 또는 SSH 화면 연결에서 기기를 확인한 뒤 다시 시도하세요"));
    }
    let mut child = std::process::Command::new(&keygen)
        .args(["-l", "-E", "sha256", "-f", "-"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()?;
    child.stdin.take().unwrap().write_all(&found)?;
    let output = child.wait_with_output()?;
    let fingerprints: Vec<_> = String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter_map(|line| line.split_whitespace().nth(1))
        .filter(|v| v.starts_with("SHA256:"))
        .map(str::to_owned)
        .collect();
    if !output.status.success() || fingerprints.is_empty() {
        return Err(error("신뢰한 SSH 키의 지문을 확인하지 못했습니다"));
    }
    Ok(fingerprints)
}
fn run(cfg: &crate::tunnel::SshCfg, command: &str, input: &[u8]) -> io::Result<String> {
    let mut cmd = crate::tunnel::ssh_command_strict(cfg)?;
    cmd.arg(crate::tunnel::ssh_target(cfg)?).arg(command);
    let mut child = cmd
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    // Drain both streams while writing to avoid deadlocking on SSH diagnostics.
    let mut stdout = child.stdout.take().unwrap();
    let mut stderr = child.stderr.take().unwrap();
    let out = std::thread::spawn(move || {
        let mut b = Vec::new();
        let _ = stdout.by_ref().take(1024 * 1024).read_to_end(&mut b);
        let _ = io::copy(&mut stdout, &mut io::sink());
        b
    });
    let err = std::thread::spawn(move || {
        let mut b = Vec::new();
        let _ = stderr.by_ref().take(1024 * 1024).read_to_end(&mut b);
        let _ = io::copy(&mut stderr, &mut io::sink());
        b
    });
    let mut stdin = child.stdin.take().unwrap();
    let data = Zeroizing::new(input.to_vec());
    let writer = std::thread::spawn(move || stdin.write_all(&data));
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(900);
    let mut timed_out = false;
    let status = loop {
        match child.try_wait() {
            Err(e) => {
                let _ = child.kill();
                let _ = child.wait();
                let _ = writer.join();
                let _ = out.join();
                let _ = err.join();
                return Err(e);
            }
            Ok(Some(status)) => break status,
            Ok(None) if std::time::Instant::now() >= deadline => {
                timed_out = true;
                let _ = child.kill();
                break child.wait()?;
            }
            Ok(None) => std::thread::sleep(std::time::Duration::from_millis(100)),
        }
    };
    let write = writer
        .join()
        .map_err(|_| error("원격 전송 작업이 중단됐습니다"))?;
    let out = out.join().unwrap_or_default();
    let err = err.join().unwrap_or_default();
    if timed_out {
        return Err(error(
            "원격 작업 응답 시간이 초과됐습니다. 원격 앱에서 적용 여부를 확인한 뒤 다시 시도하세요",
        ));
    }
    if !status.success() {
        let text = String::from_utf8_lossy(&out);
        let stderr = String::from_utf8_lossy(&err);
        // Our scripts/CLI only emit these deliberate, non-secret diagnostics.
        if let Some(line) = text
            .lines()
            .chain(stderr.lines())
            .chain(stderr.lines())
            .find(|line| line.starts_with("error="))
        {
            return Err(error(line.trim_start_matches("error=")));
        }
        if let Some(v) = stderr
            .lines()
            .find_map(|line| serde_json::from_str::<Value>(line).ok())
            .filter(|v| v["ok"] == false)
        {
            return Err(error(
                v["error"]
                    .as_str()
                    .unwrap_or("원격 설정 적용에 실패했습니다"),
            ));
        }
        // (The Windows script throws this in English: PowerShell's stderr over SSH isn't UTF-8.)
        if stderr.contains("Checksum verification failed") {
            return Err(error("내려받은 Itda 파일이 릴리스와 다릅니다 (체크섬 불일치). 잠시 뒤 다시 시도하세요"));
        }
        return Err(crate::tunnel::classify(&stderr));
    }
    write?;
    String::from_utf8(out).map_err(|_| error("원격 기기의 응답을 읽지 못했습니다"))
}
const MAC_PROBE: &str = r#"set -e
[ "$(uname -s)" = Darwin ] || { echo 'itda-unsupported-platform'; exit 0; }
version=""; versions=""
read_version() {
 local info="$1/Contents/Info.plist" v
 [ -f "$info" ] || return 0
 [ "$(/usr/libexec/PlistBuddy -c 'Print :CFBundleIdentifier' "$info" 2>/dev/null || true)" = dev.jiun.itda ] || return 0
 v=$(/usr/libexec/PlistBuddy -c 'Print :CFBundleShortVersionString' "$info" 2>/dev/null || true)
 # Only version text goes into JSON; never emit a process path or arbitrary plist text.
 case "$v" in ""|*[!0-9A-Za-z.+-]*) return 0;; esac
 [ -n "$version" ] || version="$v"
 versions="${versions}${versions:+,}\"$v\""
}
for a in /Applications/Itda.app "$HOME/Applications/Itda.app"; do read_version "$a"; done
r=false
for pid in $(pgrep -u "$(id -u)" -x itda || true); do
 r=true
 exe=$(ps -p "$pid" -o comm= 2>/dev/null || true)
 case "$exe" in */Contents/MacOS/itda) read_version "${exe%/Contents/MacOS/itda}";; esac
done
printf '{"platform":"mac","version":"%s","versions":[%s],"running":%s}\n' "$version" "$versions" "$r"
"#;
const WIN_PROBE: &str = r#"
$ErrorActionPreference='Stop'
$sid=[Security.Principal.WindowsIdentity]::GetCurrent().User.Value
$owned=@(Get-CimInstance Win32_Process -Filter "Name='itda.exe'" | Where-Object {
 # Other users' processes can deny this query. Only positively identified current-user
 # processes belong to this per-user installation; do not block it on a foreign session.
 try {
  $owner=Invoke-CimMethod -InputObject $_ -MethodName GetOwnerSid -ErrorAction Stop
  $owner.ReturnValue -eq 0 -and $owner.Sid -eq $sid
 } catch {$false}
})
$running=$owned.Count -gt 0
$exe=Join-Path $env:LOCALAPPDATA 'Programs\Itda\itda.exe'
$paths=@($exe)+@($owned | ForEach-Object {$_.ExecutablePath} | Where-Object {$_})
$version=''
$versions=@(foreach($path in ($paths | Select-Object -Unique)) {
 if(Test-Path -LiteralPath $path -PathType Leaf) {
  $v=[string](Get-Item -LiteralPath $path -ErrorAction Stop).VersionInfo.ProductVersion
  if($path -eq $exe){$version=$v}
  if(-not [string]::IsNullOrWhiteSpace($v)){$v}
 }
})
@{platform='windows';version=$version;versions=$versions;running=$running}|ConvertTo-Json -Compress
"#;
/// A newer portable app may be running outside the installer's destination. Keep the
/// highest observed version so both preparation and the final pre-install recheck refuse
/// a downgrade. Legacy probe replies without candidates still parse the same way.
fn parse_probe(line: &str) -> Option<Remote> {
    #[derive(Deserialize)]
    struct Reply {
        #[serde(flatten)]
        remote: Remote,
        #[serde(default)]
        versions: Vec<String>,
    }
    let Reply { mut remote, versions } = serde_json::from_str::<Reply>(line).ok()?;
    for version in versions {
        if crate::update::newer(&version, &remote.version) {
            remote.version = version;
        }
    }
    Some(remote)
}
fn probe(cfg: &crate::tunnel::SshCfg) -> io::Result<Remote> {
    match run(cfg, "/bin/bash -s", MAC_PROBE.as_bytes()) {
        Ok(out) => {
            if out.lines().any(|l| l == "itda-unsupported-platform") {
                return Err(error("원격 설치는 macOS와 Windows를 지원합니다"));
            }
            if let Some(r) = out
                .lines()
                .find_map(parse_probe)
            {
                return Ok(r);
            }
        }
        Err(e) if e.kind() == io::ErrorKind::PermissionDenied => return Err(e),
        Err(_) => {}
    }
    let out = run(cfg, &ps(WIN_PROBE), &[])?;
    out.lines()
        .find_map(parse_probe)
        .ok_or_else(|| {
            error("원격 설치는 macOS와 Windows를 지원합니다. SSH와 운영체제를 확인하세요")
        })
}
const WIN_VAULT_PROBE: &str = r#"$ErrorActionPreference='Stop';$ProgressPreference='SilentlyContinue'
Add-Type -TypeDefinition @'
using System;using System.Runtime.InteropServices;
public class ItdaVaultCheck {
[StructLayout(LayoutKind.Sequential,CharSet=CharSet.Unicode)] public struct C {public uint Flags,Type;public string TargetName;public IntPtr Comment;public System.Runtime.InteropServices.ComTypes.FILETIME Written;public uint Size;public IntPtr Blob;public uint Persist,Count;public IntPtr Attributes,Alias;public string User;}
[DllImport("advapi32",CharSet=CharSet.Unicode,SetLastError=true)] static extern bool CredWriteW(ref C c,uint f);
[DllImport("advapi32",CharSet=CharSet.Unicode,SetLastError=true)] static extern bool CredReadW(string t,uint type,uint f,out IntPtr p);
[DllImport("advapi32",CharSet=CharSet.Unicode,SetLastError=true)] static extern bool CredDeleteW(string t,uint type,uint f);
[DllImport("advapi32")] static extern void CredFree(IntPtr p);
public static bool Check(){string target="Itda/provision-check-"+Guid.NewGuid().ToString("N");IntPtr b=Marshal.AllocHGlobal(1),p=IntPtr.Zero;bool written=false,ok=false;
try{Marshal.WriteByte(b,120);C c=new C();c.Type=1;c.TargetName=target;c.Size=1;c.Blob=b;c.Persist=2;c.User="Itda test";written=CredWriteW(ref c,0);if(written&&CredReadW(target,1,0,out p)){C read=(C)Marshal.PtrToStructure(p,typeof(C));ok=read.Size==1&&Marshal.ReadByte(read.Blob)==120;}}
finally{if(p!=IntPtr.Zero)CredFree(p);Marshal.FreeHGlobal(b);if(written)ok=CredDeleteW(target,1,0)&&ok;}return ok;}
}
'@
@{vault_ok=[ItdaVaultCheck]::Check()}|ConvertTo-Json -Compress
"#;
fn check_remote_vault(cfg: &crate::tunnel::SshCfg) -> io::Result<()> {
    let result = run(cfg, &ps(WIN_VAULT_PROBE), &[])?;
    if result
        .lines()
        .filter_map(|s| serde_json::from_str::<Value>(s).ok())
        .any(|v| v["vault_ok"] == true)
    {
        return Ok(());
    }
    // An SSH session usually can't use Credential Manager; the account's signed-in desktop
    // session can (provisioning hands the passwords to a helper there).
    let desktop = run(cfg, &ps(WIN_DESKTOP_PROBE), &[])?;
    if desktop.lines().filter_map(|s| serde_json::from_str::<Value>(s).ok()).any(|v| v["desktop"] == true) {
        return Ok(());
    }
    Err(error("비밀번호를 저장하려면 원격 PC에 이 계정으로 로그인되어 있어야 합니다. 로그인한 뒤 다시 시도하거나, 비밀번호 공유를 해제하고 앱과 프로필만 설치하세요"))
}

/// Whether this account has a desktop session on the PC (`quser` lists it, any state).
const WIN_DESKTOP_PROBE: &str = r#"$ProgressPreference='SilentlyContinue';$u=$env:USERNAME;$n=@(quser 2>$null|Select-Object -Skip 1|Where-Object{(($_ -replace '^>','').Trim() -split '\s+')[0] -ieq $u}).Count;if($n -gt 0){[Console]::Write('{"desktop":true}')}else{[Console]::Write('{"desktop":false}')}"#;

pub fn prepare(
    cfg: crate::tunnel::SshCfg,
    choice: Selection,
    hosts: Vec<Host>,
    accounts: Vec<Account>,
    settings: Settings,
    progress: &dyn Fn(&str),
) -> io::Result<Value> {
    crate::tunnel::ssh_target(&cfg)?;
    let fingerprints = trusted_fingerprints(&cfg)?;
    // Refuse stale local bindings without SSH, secret/key reads or origin writes.
    drop(selected_profiles(&hosts, &accounts, &choice)?);
    progress("SSH 연결과 원격 기기를 확인하고 있어요…");
    let remote = probe(&cfg)?;
    if crate::update::newer(&remote.version, env!("CARGO_PKG_VERSION")) {
        return Err(error("원격 Itda가 더 최신 버전입니다. 이 기기의 Itda를 먼저 업데이트하세요. 원격 앱과 설정은 변경하지 않았습니다"));
    }
    progress("선택한 설정을 준비하고 있어요…");
    let package = snapshot(&hosts, &accounts, &settings, &choice)?;
    validate(&package)?;
    if remote.platform == "windows" && !package.secrets.is_empty() {
        check_remote_vault(&cfg)?;
    }
    let token = random_id()?;
    let summary = json!({"type":"remote_setup","prepared":true,"token":token,"platform":remote.platform,"version":remote.version,"running":remote.running,"address":cfg.host,"user":cfg.user,"port":cfg.port,"hosts":package.hosts.len(),"accounts":package.accounts.len(),"identities":package_identities(&package),"passwords":choice.passwords,"private_keys":choice.private_keys,"key_count":package.keys.len(),"settings":choice.settings,"launch":choice.launch,"close_running":choice.close_running,"fingerprints":fingerprints,"new_host_key":false});
    *PLAN.lock().unwrap() = Some(Plan {
        token: token.clone(),
        cfg,
        package,
        choice,
        remote,
        expires: std::time::Instant::now() + std::time::Duration::from_secs(300),
    });
    // A closed dialog discards immediately; an abandoned one retains no secrets indefinitely.
    std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_secs(300));
        discard(&token);
    });
    Ok(summary)
}

fn mac_install(version: &str, close: bool, launch: bool) -> String {
    let script = r#"set -e
umask 077
T=$(mktemp -d /tmp/itda-setup.XXXXXX)
trap 'rm -rf "$T"' EXIT
base=__BASE__
/usr/bin/curl -fLsS --proto '=https' --max-time 600 "$base/Itda-macos-universal.zip" -o "$T/Itda.zip"
/usr/bin/curl -fLsS --proto '=https' --max-time 30 "$base/SHA256SUMS.txt" -o "$T/sums"
want=$(awk '$2=="Itda-macos-universal.zip"{print $1}' "$T/sums")
got=$(shasum -a 256 "$T/Itda.zip" | cut -c1-64)
[ -n "$want" ] && [ "$want" = "$got" ] || { echo 'error=설치 파일의 체크섬이 일치하지 않습니다'; exit 4; }
ditto -x -k "$T/Itda.zip" "$T/unpack"
new="$T/unpack/Itda.app"
codesign --verify --deep --strict "$new" >/dev/null 2>&1 || { echo 'error=설치 앱의 서명이 올바르지 않습니다'; exit 4; }
codesign -dv --verbose=2 "$new" 2>&1 | grep -q '^TeamIdentifier=728FW73BS8$' || { echo 'error=정식 Itda 서명을 확인하지 못했습니다'; exit 4; }
[ "$(/usr/libexec/PlistBuddy -c 'Print :CFBundleShortVersionString' "$new/Contents/Info.plist")" = __VERSION__ ] || { echo 'error=설치 버전이 올바르지 않습니다'; exit 4; }
"$new/Contents/MacOS/itda" --provision-configuration __FLAGS__
"#;
    format!(
        "/bin/bash -c {}",
        q(&script
            .replace(
                "__BASE__",
                &q(&format!(
                    "https://github.com/jiunbae/itda/releases/download/v{version}"
                ))
            )
            .replace("__VERSION__", &q(version))
            .replace(
                "__FLAGS__",
                &format!(
                    "{} {}",
                    if close { "--close-running" } else { "" },
                    if launch { "--launch" } else { "" }
                )
            ))
    )
}
fn win_install(version: &str, close: bool, launch: bool) -> String {
    // Keep the UTF-16 encoded command below cmd.exe's 8191-character limit.
    // The package is passed directly to the staged native importer via stdin.
    let script = r#"$ErrorActionPreference='Stop';$ProgressPreference='SilentlyContinue'
$t=Join-Path $env:TEMP ('itda-'+[Guid]::NewGuid().ToString('N'));New-Item -ItemType Directory $t|Out-Null
try {
$b='https://github.com/jiunbae/itda/releases/download/v__VERSION__';$f=Join-Path $t 'itda.exe';$s=Join-Path $t 'sums'
Invoke-WebRequest -UseBasicParsing -TimeoutSec 600 ($b+'/itda.exe') -OutFile $f
Invoke-WebRequest -UseBasicParsing -TimeoutSec 30 ($b+'/SHA256SUMS.txt') -OutFile $s
$h=(@(Get-Content -LiteralPath $s|Where-Object {$_ -match '^[0-9a-fA-F]{64}\s+\*?itda\.exe$'})|Select-Object -First 1) -split '\s+'
if(!$h -or $h[0] -ne (Get-FileHash -LiteralPath $f -Algorithm SHA256).Hash){throw 'Checksum verification failed'}
$i=New-Object Diagnostics.ProcessStartInfo;$i.FileName=$f;$i.Arguments='--provision-configuration __FLAGS__';$i.UseShellExecute=$false;$i.CreateNoWindow=$true
$i.RedirectStandardInput=$true;$i.RedirectStandardOutput=$true;$i.RedirectStandardError=$true
$p=[Diagnostics.Process]::Start($i);$o=$p.StandardOutput.ReadToEndAsync();$e=$p.StandardError.ReadToEndAsync()
[Console]::OpenStandardInput().CopyTo($p.StandardInput.BaseStream);$p.StandardInput.Close();$p.WaitForExit()
[Console]::Out.Write($o.Result);[Console]::Error.Write($e.Result);$code=$p.ExitCode
} catch {[Console]::Error.WriteLine('error='+$_.Exception.Message);$code=1}
finally {Remove-Item -LiteralPath $t -Recurse -Force -ErrorAction SilentlyContinue}
exit $code
"#;
    ps(&script.replace("__VERSION__", version).replace(
        "__FLAGS__",
        &format!(
            "{} {}",
            if close { "--close-running" } else { "" },
            if launch { "--launch" } else { "" }
        ),
    ))
}
pub fn install(token: &str, progress: &dyn Fn(&str)) -> io::Result<Value> {
    let plan = {
        let mut stored = PLAN.lock().unwrap();
        if !stored
            .as_ref()
            .is_some_and(|p| p.token == token && std::time::Instant::now() < p.expires)
        {
            return Err(error(
                "연결 확인이 만료됐습니다. 원격 기기를 다시 확인하세요",
            ));
        }
        stored.take().unwrap()
    };
    let version = env!("CARGO_PKG_VERSION");
    if !version.bytes().all(|b| b.is_ascii_digit() || b == b'.') {
        return Err(error("설치 버전이 올바르지 않습니다"));
    }
    let current = probe(&plan.cfg)?;
    if crate::update::newer(&current.version, version) {
        return Err(error(
            "원격 Itda가 더 최신 버전으로 변경됐습니다. 이 기기를 먼저 업데이트하세요",
        ));
    }
    let raw = Zeroizing::new(serde_json::to_vec(&plan.package)?);
    if raw.len() as u64 > MAX_PACKAGE {
        return Err(error("공유 설정이 너무 큽니다"));
    }
    let command = if plan.remote.platform == "mac" {
        mac_install(version, plan.choice.close_running, plan.choice.launch)
    } else {
        win_install(version, plan.choice.close_running, plan.choice.launch)
    };
    progress("정식 앱을 확인한 뒤 원격 설치와 설정 공유를 진행하고 있어요…");
    let response = run(&plan.cfg, &command, &raw)?;
    let result = response
        .lines()
        .find_map(|line| serde_json::from_str::<Value>(line).ok())
        .filter(|v| v["ok"] == true)
        .ok_or_else(|| error("원격 설정 적용을 확인하지 못했습니다"))?;
    let mut result = json!({"hosts":result["hosts"].as_u64().ok_or_else(||error("원격 적용 결과가 올바르지 않습니다"))?,"accounts":result["accounts"].as_u64().ok_or_else(||error("원격 적용 결과가 올바르지 않습니다"))?,"ok":true,"launch_error":result["launch_error"].as_str()});
    result["type"] = json!("remote_setup");
    result["complete"] = json!(true);
    result["version"] = json!(version);
    Ok(result)
}

#[cfg(test)]
mod tests {
    #[test]
    fn fingerprint_input_reads_like_known_hosts() {
        let key = "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIA";
        assert_eq!(fingerprint_input(&format!("host {key}")), Some(format!("host {key}").as_str()));
        assert_eq!(fingerprint_input(&format!("@cert-authority *.lan {key}")), Some(format!("*.lan {key}").as_str()));
        assert_eq!(fingerprint_input(&format!("  \t@cert-authority\t *.lan {key}")), Some(format!("*.lan {key}").as_str()));
        assert_eq!(fingerprint_input(&format!(" @revoked host {key}")), None);
        assert_eq!(fingerprint_input("  # comment"), None);
        assert_eq!(fingerprint_input("   "), None);
    }

    use super::*;

    #[test]
    fn probe_keeps_the_newest_installed_or_running_version() {
        let remote = parse_probe(r#"{"platform":"windows","version":"1.0.2","running":true,"versions":["1.9.9","1.10.0","1.0.3",""]}"#).unwrap();
        assert_eq!(remote.version, "1.10.0");
        assert!(crate::update::newer(&remote.version, "1.0.3"), "a newer portable app must reach the downgrade refusal");
        let legacy = parse_probe(r#"{"platform":"mac","version":"1.0.2","running":false}"#).unwrap();
        assert_eq!(legacy.version, "1.0.2");
        let unknown = parse_probe(r#"{"platform":"windows","version":"","running":false,"versions":[]}"#).unwrap();
        assert!(unknown.version.is_empty());
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn mac_probe_finds_a_running_portable_bundle() {
        // A plain sleep executable, not Itda's GUI: no profile, clipboard, TCC or
        // LaunchServices registration. The probe only reads plist/process metadata.
        let root = std::env::temp_dir().join(format!("itda-portable-probe-{}", store::new_id()));
        let app = root.join("Portable Itda.app");
        let executable = app.join("Contents/MacOS/itda");
        fs::create_dir_all(executable.parent().unwrap()).unwrap();
        fs::copy("/bin/sleep", &executable).unwrap();
        fs::write(app.join("Contents/Info.plist"), br#"<?xml version="1.0" encoding="UTF-8"?><plist version="1.0"><dict><key>CFBundleIdentifier</key><string>dev.jiun.itda</string><key>CFBundleShortVersionString</key><string>99.98.97</string></dict></plist>"#).unwrap();
        let mut child = std::process::Command::new(&executable).arg("30").spawn().unwrap();
        let result = std::process::Command::new("/bin/bash").arg("-c").arg(MAC_PROBE).output();
        // Terminate only this owned synthetic process, never a discovered Itda.
        let _ = child.kill();
        child.wait().unwrap();
        fs::remove_dir_all(&root).unwrap();
        let output = result.unwrap();
        assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
        let remote = String::from_utf8(output.stdout).unwrap().lines().find_map(parse_probe).unwrap();
        assert!(remote.running);
        assert_eq!(remote.version, "99.98.97", "portable bundle was missed; default Apps copies must not hide it");
        assert!(crate::update::newer(&remote.version, env!("CARGO_PKG_VERSION")));
    }

    #[cfg(windows)]
    #[test]
    fn windows_probe_reads_portable_file_metadata_only_for_its_owner() {
        // The process list/ownership is controlled in this child PowerShell only;
        // ProductVersion is read from real generated PE resources, not mocked strings.
        let fixture = r#"
$ErrorActionPreference='Stop'
$ProgressPreference='SilentlyContinue'
$fixtureOwner=[Security.Principal.WindowsIdentity]::GetCurrent().User.Value
$fixtureOther=if($fixtureOwner -eq 'S-1-5-18'){'S-1-5-19'}else{'S-1-5-18'}
function New-VersionFixture($path,$type,$version) {
 [IO.Directory]::CreateDirectory([IO.Path]::GetDirectoryName($path))|Out-Null
 # Windows PowerShell compiles an .exe with an entry point; this generated fixture is
 # never executed. Only its real version resource is read below.
 $source='using System.Reflection; [assembly:AssemblyFileVersion("'+$version+'.0")] [assembly:AssemblyInformationalVersion("'+$version+'")] public class '+$type+' { public static void Main() {} }'
 Add-Type -TypeDefinition $source -OutputAssembly $path -OutputType ConsoleApplication
}
$installed=Join-Path $env:LOCALAPPDATA 'Programs\Itda\itda.exe'
$portable=Join-Path $env:LOCALAPPDATA 'Portable folder\itda.exe'
$foreign=Join-Path $env:LOCALAPPDATA 'Other user\itda.exe'
New-VersionFixture $installed 'InstalledFixture' '1.0.2'
New-VersionFixture $portable 'PortableFixture' '9.9.9'
New-VersionFixture $foreign 'OtherFixture' '99.0.0'
function Get-CimInstance {
 param($ClassName,$Filter)
 $rows=@([pscustomobject]@{ExecutablePath=$portable;FixtureSid=$fixtureOwner},
   [pscustomobject]@{ExecutablePath=$foreign;FixtureSid=$fixtureOther})
 if($fixtureDeny){$rows+=@([pscustomobject]@{ExecutablePath=$foreign;FixtureSid='denied'},[pscustomobject]@{ExecutablePath=$foreign;FixtureSid='exited'})}
 $rows
}
function Invoke-CimMethod {
 param($InputObject,$MethodName)
 if($InputObject.FixtureSid -eq 'denied'){return @{ReturnValue=2;Sid=$fixtureOwner}}
 if($InputObject.FixtureSid -eq 'exited'){throw 'fixture process exited'}
 @{ReturnValue=0;Sid=$InputObject.FixtureSid}
}
"#;
        for denied in [false, true] {
            let root = std::env::temp_dir().join(format!("itda-portable-probe-{}", store::new_id()));
            fs::create_dir_all(&root).unwrap();
            let script = format!("$fixtureDeny=${denied}\n{fixture}\n{WIN_PROBE}");
            let encoded = ps(&script);
            let result = std::process::Command::new("powershell.exe")
                .args(["-NoProfile", "-NonInteractive", "-EncodedCommand", encoded.split_whitespace().last().unwrap()])
                .env("LOCALAPPDATA", &root)
                .output();
            fs::remove_dir_all(&root).unwrap();
            let output = result.unwrap();
            assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
            let text = String::from_utf8(output.stdout).unwrap();
            let remote = text.lines().find_map(parse_probe).unwrap();
            assert!(remote.running);
            assert_eq!(remote.version, "9.9.9", "owned portable version must win, foreign/denied/exited processes must not");
            assert!(crate::update::newer(&remote.version, env!("CARGO_PKG_VERSION")));
        }
    }

    #[cfg(windows)]
    #[test]
    fn windows_bootstrap_is_valid_powershell_without_executing_it() {
        for command in [win_install("0.9.99", true, true), ps(WIN_VAULT_PROBE), ps(WIN_PROBE)] {
            let encoded = command.split_whitespace().last().unwrap();
            let mut child=std::process::Command::new("powershell.exe").args(["-NoProfile","-NonInteractive","-Command",r#"$s=[Text.Encoding]::Unicode.GetString([Convert]::FromBase64String([Console]::In.ReadToEnd()));$t=$null;$e=$null;[Management.Automation.Language.Parser]::ParseInput($s,[ref]$t,[ref]$e)|Out-Null;if($e.Count){$e|ForEach-Object {[Console]::Error.WriteLine($_.Message)};exit 1}"#]).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped()).spawn().unwrap();
            child
                .stdin
                .take()
                .unwrap()
                .write_all(encoded.as_bytes())
                .unwrap();
            let result = child.wait_with_output().unwrap();
            assert!(
                result.status.success(),
                "{}",
                String::from_utf8_lossy(&result.stderr)
            );
        }
        let compile = ps(&WIN_VAULT_PROBE.replace(
            "@{vault_ok=[ItdaVaultCheck]::Check()}|ConvertTo-Json -Compress",
            "",
        ));
        let output = std::process::Command::new("powershell.exe")
            .args([
                "-NoProfile",
                "-NonInteractive",
                "-EncodedCommand",
                compile.split_whitespace().last().unwrap(),
            ])
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    #[test]
    fn installers_keep_secrets_on_stdin_and_windows_command_fits_cmd_limit() {
        let win = win_install("0.9.99", true, true);
        assert!(ps(WIN_VAULT_PROBE).len() < 8191);
        assert!(
            win.len() < 8191,
            "encoded Windows command is {} characters",
            win.len()
        );
        assert!(!win.contains("password"));
        let mac = mac_install("0.9.99", true, true);
        assert!(mac.starts_with("/bin/bash -c "));
        assert!(mac.contains("--provision-configuration --close-running --launch"));
        assert!(!mac.contains("mv "));
    }
    #[test]
    fn failed_app_swap_preserves_original_and_success_preserves_backup() {
        let dir = Scratch::new();
        let target = dir.0.join("app");
        let backup = dir.0.join("backup");
        let stage = dir.0.join("new");
        fs::write(&target, b"old").unwrap();
        assert!(replace_staged(&stage, &target, &backup).is_err());
        assert_eq!(fs::read(&target).unwrap(), b"old");
        assert!(!backup.exists());
        fs::write(&stage, b"new").unwrap();
        replace_staged(&stage, &target, &backup).unwrap();
        assert_eq!(fs::read(&target).unwrap(), b"new");
        assert_eq!(fs::read(&backup).unwrap(), b"old");
        // Failed preservation must not delete the installed app.
        fs::create_dir(dir.0.join("nonempty")).unwrap();
        fs::write(dir.0.join("nonempty/child"), b"x").unwrap();
        fs::write(&stage, b"newer").unwrap();
        assert!(replace_staged(&stage, &target, &dir.0.join("nonempty")).is_err());
        assert_eq!(fs::read(&target).unwrap(), b"new");
    }
    #[derive(Default)]
    struct FakeVault {
        data: HashMap<String, String>,
        calls: usize,
        fail_at: usize,
        fail_get: bool,
    }
    impl Vault for FakeVault {
        fn get(&self, key: &str) -> io::Result<Option<String>> {
            if self.fail_get {
                return Err(error("test vault inaccessible"));
            }
            Ok(self.data.get(key).cloned())
        }
        fn set(&mut self, key: &str, value: Option<&str>) -> io::Result<()> {
            self.calls += 1;
            match value {
                Some(value) => {
                    self.data.insert(key.into(), value.into());
                }
                None => {
                    self.data.remove(key);
                }
            }
            if self.calls == self.fail_at {
                Err(error("test vault failure"))
            } else {
                Ok(())
            }
        }
    }
    #[test]
    fn inaccessible_vault_is_not_treated_as_a_missing_rollback_value() {
        let dir = Scratch::new();
        let mut p = package();
        p.secrets.push(Secret {
            kind: SecretKind::Account,
            owner: "source-account".into(),
            value: "new-test-value".into(),
        });
        let mut vault = FakeVault {
            fail_get: true,
            ..Default::default()
        };
        let key = store::acct_key(&mapped(&p.origin, "account", "source-account"));
        vault.data.insert(key.clone(), "keep-test-value".into());
        assert!(apply(&dir.0, &p, &mut vault).is_err());
        assert_eq!(vault.calls, 0);
        assert_eq!(vault.data[&key], "keep-test-value");
        assert!(!dir.0.join("hosts.json").exists());
    }
    struct Scratch(PathBuf);
    impl Scratch {
        fn new() -> Self {
            let root = std::env::temp_dir().join(format!(
                "itda-configuration-test-{}-{}",
                std::process::id(),
                store::new_id()
            ));
            fs::create_dir(&root).unwrap();
            Self(root)
        }
        fn write(&self, name: &str, value: &impl Serialize) {
            fs::write(self.0.join(name), serde_json::to_vec(value).unwrap()).unwrap();
        }
    }
    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }
    fn package() -> Package {
        let mut preferences = Settings::default();
        preferences.theme = "dark".into();
        Package {
            schema: SCHEMA,
            origin: "source-device".into(),
            hosts: vec![Host {
                id: "source-host".into(),
                name: "Shared Mac".into(),
                host: "mac.example".into(),
                account: "source-account".into(),
                ..Host::default()
            }],
            accounts: vec![Account {
                id: "source-account".into(),
                name: "Shared account".into(),
                user: "shared-user".into(),
                ssh_key: "/never/import/source/path".into(),
            }],
            preferences: Some(Preferences::from(&preferences)),
            secrets: vec![],
            keys: vec![],
        }
    }
    fn empty_package() -> Package {
        Package { schema: SCHEMA, origin: "app-only-test".into(), hosts: vec![], accounts: vec![], preferences: None, secrets: vec![], keys: vec![] }
    }
    struct NoVault;
    impl Vault for NoVault {
        fn get(&self, _: &str) -> io::Result<Option<String>> { panic!("unselected vault was read") }
        fn set(&mut self, _: &str, _: Option<&str>) -> io::Result<()> { panic!("unselected vault was written") }
    }
    fn raw_state(dir: &Scratch, name: &str) -> (Option<Vec<u8>>, Option<bool>, Option<u32>) {
        let path = dir.0.join(name);
        let metadata = fs::metadata(&path).ok();
        #[cfg(unix)]
        let mode = { use std::os::unix::fs::PermissionsExt; metadata.as_ref().map(|m| m.permissions().mode()) };
        #[cfg(not(unix))]
        let mode = None;
        (fs::read(path).ok(), metadata.map(|m| m.permissions().readonly()), mode)
    }
    fn write_raw(dir: &Scratch, name: &str, bytes: &[u8]) {
        fs::write(dir.0.join(name), bytes).unwrap();
        #[cfg(unix)]
        { use std::os::unix::fs::PermissionsExt; fs::set_permissions(dir.0.join(name), fs::Permissions::from_mode(0o640)).unwrap(); }
    }
    #[test]
    fn preservation_empty_package_never_reads_or_rewrites_configuration() {
        for invalid in [false, true] {
            let dir = Scratch::new();
            for name in ["hosts.json", "accounts.json", "settings.json"] {
                let valid: &[u8] = match name {
                    "hosts.json" => b"[ { \"id\":\"local\", \"future-field\": {\"keep\":42} } ]\n",
                    "accounts.json" => b"[ { \"id\":\"local\", \"future-field\": {\"keep\":42} } ]\n",
                    _ => b"{ \"theme\":\"light\", \"future-field\": {\"keep\":42} }\n",
                };
                write_raw(&dir, name, if invalid { b"not JSON\0\n" } else { valid });
            }
            let before: Vec<_> = ["hosts.json", "accounts.json", "settings.json"].map(|n| raw_state(&dir, n)).into();
            assert_eq!(apply(&dir.0, &empty_package(), &mut NoVault).unwrap(), json!({"ok":true,"hosts":0,"accounts":0,"secrets":0,"keys":0}));
            let after: Vec<_> = ["hosts.json", "accounts.json", "settings.json"].map(|n| raw_state(&dir, n)).into();
            assert_eq!(after, before);
            assert!(!dir.0.join("configuration-backups").exists());
            assert!(!dir.0.join("imported-keys").exists());
        }
        let dir = Scratch::new();
        apply(&dir.0, &empty_package(), &mut NoVault).unwrap();
        for name in ["hosts.json", "accounts.json", "settings.json", "configuration-backups"] { assert!(!dir.0.join(name).exists(), "no-op created {name}"); }
        // No-op is still behind validation and the real configuration lock.
        let mut invalid = empty_package(); invalid.schema = SCHEMA + 1;
        assert!(apply(&dir.0, &invalid, &mut NoVault).is_err());
        invalid.schema = SCHEMA; invalid.origin.clear();
        assert!(apply(&dir.0, &invalid, &mut NoVault).is_err());
        let _live = lock_at(&dir.0).unwrap();
        assert!(apply(&dir.0, &empty_package(), &mut NoVault).is_err());
    }
    #[test]
    fn preservation_secret_owners_are_validated_before_any_side_effect() {
        for remove_section in [true, false] {
            for case in 0..5 {
                let dir = Scratch::new();
                // Even selected malformed JSON must not be read before validation.
                for name in ["hosts.json", "accounts.json", "settings.json"] { write_raw(&dir, name, b"invalid existing fixture JSON\n"); }
                let before: Vec<_> = ["hosts.json", "accounts.json", "settings.json"].map(|n| raw_state(&dir, n)).into();
                let mut p = package();
                let account = case < 2;
                if remove_section { if account { p.accounts.clear(); } else { p.hosts.clear(); } }
                if !p.accounts.is_empty() { p.keys.push(Key { account: "source-account".into(), bytes: b"generated-unused-key-fixture".to_vec() }); }
                let kind = match case { 0 => SecretKind::Account, 1 => SecretKind::KeyPass, 2 => SecretKind::Host, 3 => SecretKind::Hp, _ => SecretKind::Files };
                let owner = if !remove_section { "not-selected" } else if account { "source-account" } else { "source-host" };
                p.secrets.push(Secret { kind, owner: owner.into(), value: "generated-invalid-owner-fixture".into() });
                let error = apply(&dir.0, &p, &mut NoVault).unwrap_err();
                assert!(error.to_string().contains("암호의 계정 또는 연결"), "configuration was parsed before secret dependency validation: {error}");
                let after: Vec<_> = ["hosts.json", "accounts.json", "settings.json"].map(|n| raw_state(&dir, n)).into();
                assert_eq!(after, before);
                assert!(!dir.0.join("configuration.lock").exists());
                assert!(!dir.0.join("configuration-backups").exists());
                assert!(!dir.0.join("imported-keys").exists());
                assert_eq!(fs::read_dir(&dir.0).unwrap().count(), 3);
            }
        }
    }
    #[test]
    fn preservation_valid_selected_secret_dependencies_still_import() {
        let dir = Scratch::new();
        let mut p = package();
        for (kind, owner) in [(SecretKind::Account, "source-account"), (SecretKind::KeyPass, "source-account"), (SecretKind::Host, "source-host"), (SecretKind::Hp, "source-host"), (SecretKind::Files, "source-host")] {
            p.secrets.push(Secret { kind, owner: owner.into(), value: "generated-selected-secret-fixture".into() });
        }
        let mut vault = FakeVault::default();
        let result = apply(&dir.0, &p, &mut vault).unwrap();
        assert_eq!(result["secrets"], 5);
        assert_eq!(vault.calls, 5);
        assert_eq!(read_json::<Vec<Host>>(&dir.0, "hosts.json").unwrap().len(), 1);
        assert_eq!(read_json::<Vec<Account>>(&dir.0, "accounts.json").unwrap().len(), 1);
    }
    #[test]
    fn missing_account_bindings_fail_before_destination_reads_or_any_side_effect() {
        for files_binding in [false,true] {
            for destination_has_account in [false,true] {
                let dir=Scratch::new();
                for name in ["hosts.json","accounts.json","settings.json"] {write_raw(&dir,name,b"invalid existing fixture JSON\n");}
                if destination_has_account {
                    dir.write("accounts.json",&vec![Account {id:"missing-package-account".into(),user:"destination-only-user".into(),..Account::default()}]);
                }
                let before:Vec<_>=["hosts.json","accounts.json","settings.json"].map(|n|raw_state(&dir,n)).into();
                let mut p=package();
                let expected=if files_binding {
                    p.hosts[0].files.account="missing-package-account".into();
                    "‘Shared Mac’의 파일 연결 계정을 찾을 수 없습니다. 파일 열기에서 계정을 다시 선택한 뒤 공유해 주세요."
                }else{
                    p.hosts[0].account="missing-package-account".into();
                    "‘Shared Mac’의 연결 계정을 찾을 수 없습니다. 연결을 편집해 계정을 다시 선택한 뒤 공유해 주세요."
                };
                // These valid owners would cause key/Vault mutations if validation
                // were delayed until account remapping or configuration writes.
                p.keys.push(Key {account:"source-account".into(),bytes:b"unused-generated-key-fixture".to_vec()});
                p.secrets.push(Secret {kind:SecretKind::Account,owner:"source-account".into(),value:"unused-generated-secret-fixture".into()});
                assert_eq!(apply(&dir.0,&p,&mut NoVault).unwrap_err().to_string(),expected);
                let after:Vec<_>=["hosts.json","accounts.json","settings.json"].map(|n|raw_state(&dir,n)).into();
                assert_eq!(after,before);
                // No lock, backup/.next staging, imported key or other new entry.
                assert_eq!(fs::read_dir(&dir.0).unwrap().count(),3);
                for name in ["configuration.lock","configuration-backups","imported-keys"] {assert!(!dir.0.join(name).exists());}
            }
        }
    }
    #[test]
    fn valid_empty_and_packaged_account_bindings_keep_their_import_semantics() {
        let dir=Scratch::new();
        write_raw(&dir,"accounts.json",b"unselected destination account bytes\n");
        let before=raw_state(&dir,"accounts.json");
        let mut p=package();p.preferences=None;p.accounts.clear();p.hosts[0].account.clear();
        apply(&dir.0,&p,&mut NoVault).unwrap();
        let hosts:Vec<Host>=read_json(&dir.0,"hosts.json").unwrap();
        assert!(hosts[0].account.is_empty() && hosts[0].files.account.is_empty());
        assert_eq!(raw_state(&dir,"accounts.json"),before);

        let dir=Scratch::new();
        let mut p=package();p.preferences=None;
        p.accounts.push(Account {id:"file-account".into(),user:"packaged-file-user".into(),..Account::default()});
        p.hosts[0].files.account="file-account".into();
        dir.write("accounts.json",&vec![Account {id:"file-account".into(),user:"destination-only-user".into(),..Account::default()}]);
        apply(&dir.0,&p,&mut NoVault).unwrap();
        let hosts:Vec<Host>=read_json(&dir.0,"hosts.json").unwrap();
        let accounts:Vec<Account>=read_json(&dir.0,"accounts.json").unwrap();
        assert_eq!(hosts[0].account,mapped(&p.origin,"account","source-account"));
        assert_eq!(hosts[0].files.account,mapped(&p.origin,"account","file-account"));
        assert_eq!(accounts.iter().find(|a|a.id==hosts[0].files.account).unwrap().user,"packaged-file-user");
        assert_eq!(accounts.iter().find(|a|a.id=="file-account").unwrap().user,"destination-only-user");
    }
    #[test]
    fn custom_file_login_snapshot_import_preserves_user_and_separate_secret_routing() {
        let mut selected=package();
        selected.hosts[0].files.account="custom".into();
        selected.hosts[0].files.user="generated-file-user".into();
        selected.hosts[0].files.host="fictional.invalid".into();
        let choice=Selection {host_ids:vec!["source-host".into()],..Selection::default()};
        // No native credential-store/key reads, settings directory or source
        // origin file: only the snapshot's origin provider is replaced.
        let mut p=snapshot_with_origin(&selected.hosts,&selected.accounts,&Settings::default(),&choice,||Ok("generated-custom-source".into())).unwrap();
        assert_eq!(p.accounts.len(),1);
        assert_eq!(p.accounts[0].id,"source-account");
        assert!(p.keys.is_empty() && p.secrets.is_empty());
        assert_eq!(p.hosts[0].files.account,"custom");
        p.secrets.push(Secret {kind:SecretKind::Files,owner:"source-host".into(),value:"generated-custom-files-secret".into()});
        let dir=Scratch::new();let mut vault=FakeVault::default();
        apply(&dir.0,&p,&mut vault).unwrap();
        let hosts:Vec<Host>=read_json(&dir.0,"hosts.json").unwrap();
        assert_eq!(hosts[0].files.account,"custom");
        assert_eq!(hosts[0].files.user,"generated-file-user");
        assert_eq!(hosts[0].account,mapped(&p.origin,"account","source-account"));
        let expected=format!("files-{}-custom-generated-file-user",mapped(&p.origin,"host","source-host"));
        assert_eq!(crate::files::password_key(&hosts[0]),expected);
        assert_eq!(vault.data,HashMap::from([(expected,"generated-custom-files-secret".into())]));
        assert_eq!(vault.calls,1);
    }
    #[test]
    fn snapshot_missing_source_account_fails_before_origin_and_keeps_binding() {
        for file_binding in [false, true] {
            let mut source = package();
            source.hosts[0].name = "<b>공유 & 연결</b>".into();
            if file_binding {
                source.hosts[0].files.account = "deleted-account".into();
            } else {
                source.hosts[0].account = "deleted-account".into();
            }
            let choice = Selection {
                host_ids: vec!["source-host".into()],
                passwords: false,
                private_keys: false,
                ..Selection::default()
            };
            let error = snapshot_with_origin(
                &source.hosts,
                &source.accounts,
                &Settings::default(),
                &choice,
                || panic!("invalid source reached the origin provider"),
            )
            .err()
            .expect("a missing source account must not produce a package")
            .to_string();
            assert!(error.contains("<b>공유 & 연결</b>"));
            assert!(error.contains("계정을 다시 선택"));
            if file_binding {
                assert!(error.contains("파일 열기에서"));
                assert_eq!(source.hosts[0].files.account, "deleted-account");
            } else {
                assert!(error.contains("연결을 편집해"));
                assert_eq!(source.hosts[0].account, "deleted-account");
            }
            assert_eq!(source.accounts[0].id, "source-account");
        }
    }
    #[test]
    fn preservation_host_only_leaves_unselected_bytes_modes_and_absence() {
        for variant in 0..3 {
            let dir = Scratch::new();
            write_raw(&dir, "hosts.json", br#"[{"id":"existing","host":"retained.example"}]"#);
            if variant != 2 {
                write_raw(&dir, "settings.json", if variant == 0 { b"{ \"theme\":\"light\", \"future\":{\"keep\":true} }\n" } else { b"invalid settings JSON\n" });
                write_raw(&dir, "accounts.json", if variant == 0 { b"[ { \"id\":\"existing\", \"future\":17 } ]\n" } else { b"invalid accounts JSON\n" });
            }
            let settings = raw_state(&dir, "settings.json");
            let accounts = raw_state(&dir, "accounts.json");
            let mut p = package(); p.accounts.clear(); p.hosts[0].account.clear(); p.preferences = None;
            let result = apply(&dir.0, &p, &mut NoVault).unwrap();
            assert_eq!(result["hosts"], 1);
            assert_eq!(raw_state(&dir, "settings.json"), settings);
            assert_eq!(raw_state(&dir, "accounts.json"), accounts);
            let hosts: Vec<Host> = read_json(&dir.0, "hosts.json").unwrap();
            assert_eq!(hosts.len(), 2);
            assert!(hosts.iter().any(|h| h.id == mapped(&p.origin, "host", "source-host") && h.host == "mac.example"));
            let backups = fs::read_dir(dir.0.join("configuration-backups")).unwrap().collect::<Result<Vec<_>, _>>().unwrap();
            assert_eq!(backups.len(), 1);
            let names: Vec<_> = fs::read_dir(backups[0].path()).unwrap().map(|e| e.unwrap().file_name()).collect();
            assert_eq!(names, [std::ffi::OsString::from("hosts.json")]);
        }
    }
    #[test]
    fn preservation_selected_rollback_does_not_touch_opted_out_configs() {
        let dir = Scratch::new();
        write_raw(&dir, "hosts.json", br#"[{"id":"local","host":"retained.example"}]"#);
        write_raw(&dir, "accounts.json", b"invalid unselected accounts\n");
        write_raw(&dir, "settings.json", b"{ \"future-setting\":42 }\n");
        let before: Vec<_> = ["hosts.json", "accounts.json", "settings.json"].map(|n| raw_state(&dir, n)).into();
        let mut p = package(); p.accounts.clear(); p.hosts[0].account.clear(); p.preferences = None;
        p.secrets.push(Secret { kind: SecretKind::Host, owner: "source-host".into(), value: "generated-rollback-fixture".into() });
        let mut vault = FakeVault { fail_at: 1, ..Default::default() };
        assert!(apply(&dir.0, &p, &mut vault).is_err());
        assert!(vault.data.is_empty() && vault.calls == 2, "selected secret was not rolled back");
        let after: Vec<_> = ["hosts.json", "accounts.json", "settings.json"].map(|n| raw_state(&dir, n)).into();
        assert_eq!(after, before);
        let backup = fs::read_dir(dir.0.join("configuration-backups")).unwrap().next().unwrap().unwrap().path();
        assert!(!backup.join("accounts.json").exists() && !backup.join("settings.json").exists());
        assert!(!backup.join("accounts.json.next").exists() && !backup.join("settings.json.next").exists());
    }
    #[test]
    fn preservation_preferences_only_applies_real_preferences_without_other_writes() {
        let dir = Scratch::new();
        write_raw(&dir, "hosts.json", b"unknown/corrupt unselected hosts\n");
        write_raw(&dir, "settings.json", br#"{"theme":"light","web":{"on":true,"port":9123},"announce":false}"#);
        let hosts = raw_state(&dir, "hosts.json");
        let accounts = raw_state(&dir, "accounts.json");
        let mut p = empty_package();
        let mut prefs = Settings::default(); prefs.theme = "dark".into(); prefs.zoom = 125;
        p.preferences = Some(Preferences::from(&prefs));
        apply(&dir.0, &p, &mut NoVault).unwrap();
        let actual: Settings = read_json(&dir.0, "settings.json").unwrap();
        assert_eq!(actual.theme, "dark"); assert_eq!(actual.zoom, 125);
        assert!(actual.web.on && actual.web.port == 9123 && !actual.announce);
        assert_eq!(raw_state(&dir, "hosts.json"), hosts);
        assert_eq!(raw_state(&dir, "accounts.json"), accounts);
        let backup = fs::read_dir(dir.0.join("configuration-backups")).unwrap().next().unwrap().unwrap().path();
        let names: Vec<_> = fs::read_dir(backup).unwrap().map(|e| e.unwrap().file_name()).collect();
        assert_eq!(names, [std::ffi::OsString::from("settings.json")]);
    }
    #[test]
    fn preservation_accounts_only_does_not_parse_unselected_host_or_settings() {
        let dir = Scratch::new();
        write_raw(&dir, "hosts.json", b"invalid hosts\n");
        write_raw(&dir, "settings.json", b"{ \"future-setting\":42 }\n");
        let hosts = raw_state(&dir, "hosts.json"); let settings = raw_state(&dir, "settings.json");
        let mut p = package(); p.hosts.clear(); p.preferences = None;
        apply(&dir.0, &p, &mut NoVault).unwrap();
        let accounts: Vec<Account> = read_json(&dir.0, "accounts.json").unwrap();
        assert_eq!(accounts.len(), 1); assert_eq!(accounts[0].user, "shared-user");
        assert_eq!(raw_state(&dir, "hosts.json"), hosts); assert_eq!(raw_state(&dir, "settings.json"), settings);
    }

    #[test]
    fn confirmation_identities_follow_packaged_accounts_without_secrets() {
        let mut p = package();
        p.accounts[0].id = "replacement-account".into();
        p.accounts[0].name = "Actual account".into();
        p.accounts[0].user = "actual-user".into();
        p.hosts[0].account = p.accounts[0].id.clone();
        // The source key path does not prove a key was packaged.
        assert_eq!(package_identities(&p)["accounts"][0]["key"], false);
        p.keys.push(Key { account: p.accounts[0].id.clone(), bytes: b"PRIVATE-KEY-CONTENT".to_vec() });
        p.secrets.push(Secret { kind: SecretKind::Account, owner: p.accounts[0].id.clone(), value: "PASSWORD-CONTENT".into() });
        let identities = package_identities(&p);
        assert_eq!(identities["hosts"][0]["account"], "Actual account");
        assert_eq!(identities["accounts"][0]["id"], "replacement-account");
        assert_eq!(identities["accounts"][0]["user"], "actual-user");
        assert_eq!(identities["accounts"][0]["key"], true);
        assert_eq!(identities["accounts"][0]["password"], true);
        let rendered = identities.to_string();
        for secret in ["PRIVATE-KEY-CONTENT", "PASSWORD-CONTENT", "/never/import/source/path"] {
            assert!(!rendered.contains(secret));
        }
    }
    #[test]
    fn merge_is_idempotent_preserves_unrelated_profiles_and_machine_sharing() {
        let dir = Scratch::new();
        let p = package();
        let existing = Host {
            id: "source-host".into(),
            host: "untouched.example".into(),
            ..Host::default()
        };
        dir.write("hosts.json", &vec![existing]);
        dir.write(
            "accounts.json",
            &vec![Account {
                id: "source-account".into(),
                user: "untouched".into(),
                ssh_key: "destination-key".into(),
                ..Account::default()
            }],
        );
        let mut settings = Settings::default();
        settings.web.on = true;
        settings.web.port = 9999;
        settings.share.on = true;
        settings.announce = false;
        dir.write("settings.json", &settings);
        let mut vault = FakeVault::default();
        vault
            .data
            .insert("unrelated".into(), "not-for-export".into());
        apply(&dir.0, &p, &mut vault).unwrap();
        apply(&dir.0, &p, &mut vault).unwrap();
        let hosts: Vec<Host> = read_json(&dir.0, "hosts.json").unwrap();
        let accounts: Vec<Account> = read_json(&dir.0, "accounts.json").unwrap();
        assert_eq!(hosts.len(), 2);
        assert_eq!(accounts.len(), 2);
        assert_eq!(hosts[0].host, "untouched.example");
        let new_account = mapped(&p.origin, "account", "source-account");
        assert_eq!(hosts[1].account, new_account);
        assert_eq!(accounts[1].id, new_account);
        assert_eq!(accounts[1].ssh_key, "");
        let s: Settings = read_json(&dir.0, "settings.json").unwrap();
        assert_eq!(s.theme, "dark");
        assert!(s.web.on && s.share.on);
        assert_eq!(s.web.port, 9999);
        assert!(!s.announce);
        assert_eq!(vault.calls, 0);
        assert_eq!(vault.data.get("unrelated").unwrap(), "not-for-export");
    }
    #[test]
    fn vault_failure_restores_credentials_configs_and_new_keys() {
        let dir = Scratch::new();
        let mut p = package();
        dir.write(
            "hosts.json",
            &vec![Host {
                id: "old".into(),
                ..Host::default()
            }],
        );
        let original = fs::read(dir.0.join("hosts.json")).unwrap();
        p.secrets = vec![
            Secret {
                kind: SecretKind::Account,
                owner: "source-account".into(),
                value: "example-only-account-password".into(),
            },
            Secret {
                kind: SecretKind::Host,
                owner: "source-host".into(),
                value: "example-only-host-password".into(),
            },
        ];
        p.keys.push(Key {
            account: "source-account".into(),
            bytes: b"test-only-private-key".to_vec(),
        });
        let mut vault = FakeVault {
            fail_at: 2,
            ..FakeVault::default()
        };
        vault.data.insert("unrelated".into(), "keep".into());
        assert!(apply(&dir.0, &p, &mut vault).is_err());
        assert_eq!(fs::read(dir.0.join("hosts.json")).unwrap(), original);
        assert!(!dir.0.join("accounts.json").exists());
        assert_eq!(
            vault.data,
            HashMap::from([("unrelated".into(), "keep".into())])
        );
        assert_eq!(
            fs::read_dir(dir.0.join("imported-keys")).unwrap().count(),
            0
        );
    }
    #[test]
    fn key_copy_uses_destination_path_and_credentials_never_enter_json_files() {
        let dir = Scratch::new();
        let mut p = package();
        p.keys.push(Key {
            account: "source-account".into(),
            bytes: b"test-only-private-key".to_vec(),
        });
        p.secrets.push(Secret {
            kind: SecretKind::Account,
            owner: "source-account".into(),
            value: "test-only-vault-secret".into(),
        });
        let mut vault = FakeVault::default();
        apply(&dir.0, &p, &mut vault).unwrap();
        let accounts: Vec<Account> = read_json(&dir.0, "accounts.json").unwrap();
        let key = PathBuf::from(&accounts[0].ssh_key);
        assert!(key.starts_with(dir.0.join("imported-keys")));
        assert_eq!(fs::read(&key).unwrap(), b"test-only-private-key");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                fs::metadata(&key).unwrap().permissions().mode() & 0o777,
                0o600
            );
            assert_eq!(
                fs::metadata(key.parent().unwrap())
                    .unwrap()
                    .permissions()
                    .mode()
                    & 0o777,
                0o700
            );
        }
        for name in ["hosts.json", "accounts.json", "settings.json"] {
            let bytes = fs::read_to_string(dir.0.join(name)).unwrap();
            assert!(!bytes.contains("test-only-vault-secret"));
            assert!(!bytes.contains("test-only-private-key"));
        }
        assert_eq!(
            vault.data.get(&store::acct_key(&accounts[0].id)).unwrap(),
            "test-only-vault-secret"
        );
        // Omitting keys on subsequent pushes preserves the destination's imported key.
        p.keys.clear();
        apply(&dir.0, &p, &mut vault).unwrap();
        let after: Vec<Account> = read_json(&dir.0, "accounts.json").unwrap();
        assert_eq!(after[0].ssh_key, accounts[0].ssh_key);
    }
    #[test]
    fn corrupt_existing_data_and_running_app_lock_are_not_overwritten() {
        let dir = Scratch::new();
        let p = package();
        let mut vault = FakeVault::default();
        fs::write(dir.0.join("hosts.json"), b"invalid existing JSON").unwrap();
        assert!(apply(&dir.0, &p, &mut vault).is_err());
        assert_eq!(
            fs::read(dir.0.join("hosts.json")).unwrap(),
            b"invalid existing JSON"
        );
        fs::remove_file(dir.0.join("hosts.json")).unwrap();
        let _live = lock_at(&dir.0).unwrap();
        assert!(apply(&dir.0, &p, &mut vault).is_err());
        assert_eq!(vault.calls, 0);
    }
    #[cfg(unix)]
    #[test]
    fn private_keys_are_not_written_through_a_directory_symlink() {
        let dir = Scratch::new();
        let outside = dir.0.join("outside");
        fs::create_dir(&outside).unwrap();
        std::os::unix::fs::symlink(&outside, dir.0.join("imported-keys")).unwrap();
        let mut p = package();
        p.keys.push(Key {
            account: "source-account".into(),
            bytes: b"test-only".to_vec(),
        });
        assert!(apply(&dir.0, &p, &mut FakeVault::default()).is_err());
        assert_eq!(fs::read_dir(outside).unwrap().count(), 0);
    }
}

#[cfg(all(test, target_os = "macos"))]
mod session_vault_tests {
    use super::*;
    use std::os::unix::net::UnixStream;

    #[derive(Default)]
    struct Memory(HashMap<String, String>);
    impl Vault for Memory {
        fn get(&self, key: &str) -> io::Result<Option<String>> {
            Ok(self.0.get(key).cloned())
        }
        fn set(&mut self, key: &str, value: Option<&str>) -> io::Result<()> {
            if key == "refused" {
                return Err(error("키체인이 거부했습니다"));
            }
            match value {
                Some(v) => self.0.insert(key.into(), v.into()),
                None => self.0.remove(key),
            };
            Ok(())
        }
    }
    fn linked(peer: UnixStream) -> SessionVault {
        let v = SessionVault::new(PathBuf::new());
        peer.set_read_timeout(Some(std::time::Duration::from_secs(2))).unwrap();
        *v.link.borrow_mut() = Some(peer);
        v
    }

    /// Set, read back, delete through the real helper loop; a refusal is an answer, not a
    /// broken channel.
    #[test]
    fn helper_round_trip() {
        let (a, mut b) = UnixStream::pair().unwrap();
        let helper = std::thread::spawn(move || serve_vault(&mut b, &mut Memory::default()));
        let mut v = linked(a);
        v.set("k", Some("비밀 🔑")).unwrap();
        assert_eq!(v.get("k").unwrap().as_deref(), Some("비밀 🔑"));
        assert!(v.set("refused", Some("x")).is_err());
        assert!(!v.poisoned.get());
        v.set("k", None).unwrap();
        assert_eq!(v.get("k").unwrap(), None);
        drop(v);
        helper.join().unwrap().unwrap();
    }

    /// A reply for another request, a cut-off reply or a closed helper poison the channel:
    /// later requests (a rollback) fail instead of reading a stale answer.
    #[test]
    fn unknown_outcome_poisons() {
        for reply in [&b"{\"id\":7,\"ok\":true}\n"[..], b"{\"id\":1,\"ok\"", b""] {
            let (a, mut b) = UnixStream::pair().unwrap();
            let reply = reply.to_vec();
            let peer = std::thread::spawn(move || {
                let _ = read_line_secret(&mut b);
                b.write_all(&reply).unwrap();
                drop(b);
            });
            let mut v = linked(a);
            assert!(v.set("k", Some("x")).is_err());
            peer.join().unwrap();
            assert!(v.poisoned.get());
            assert!(v.set("k", None).is_err(), "no request after an unknown outcome");
        }
    }
}

/// Live check on a Windows PC over SSH, with this account signed in at the PC: a test entry
/// goes through the desktop-session helper into Credential Manager and is removed again.
/// cargo build --release; cargo test --release desktop_helper_round_trip -- --ignored
#[cfg(all(test, windows))]
mod windows_vault_live {
    use super::*;
    #[test]
    #[ignore]
    fn desktop_helper_round_trip() {
        let exe = std::env::current_dir().unwrap().join("target/release/itda.exe");
        assert!(exe.exists(), "build the release app first");
        let mut v = SessionVault::new(exe);
        let key = format!("provision-live-{}", &random_id().unwrap()[..8]);
        v.set(&key, Some("비밀 🔑")).unwrap();
        assert_eq!(v.get(&key).unwrap().as_deref(), Some("비밀 🔑"));
        v.set(&key, None).unwrap();
        assert_eq!(v.get(&key).unwrap(), None);
    }
}
