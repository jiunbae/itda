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

pub fn snapshot(
    hosts: &[Host],
    accounts: &[Account],
    settings: &Settings,
    choice: &Selection,
) -> io::Result<Package> {
    let selected: HashSet<_> = choice.host_ids.iter().collect();
    if selected.len() != choice.host_ids.len()
        || selected
            .iter()
            .any(|id| !hosts.iter().any(|h| &h.id == *id))
    {
        return Err(error("공유할 연결을 다시 선택해 주세요"));
    }
    let mut selected_hosts: Vec<_> = hosts
        .iter()
        .filter(|h| selected.contains(&h.id))
        .cloned()
        .collect();
    let references: HashSet<_> = selected_hosts
        .iter()
        .flat_map(|h| [h.account.as_str(), h.files.account.as_str()])
        .collect();
    let mut selected_accounts: Vec<_> = accounts
        .iter()
        .filter(|a| references.contains(a.id.as_str()))
        .cloned()
        .collect();
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
        origin: origin(&store::dir())?,
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
        let ok = Command::new("powershell.exe")
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
    let mut hosts: Vec<Host> = read_json(root, "hosts.json")?;
    let mut accounts: Vec<Account> = read_json(root, "accounts.json")?;
    let mut settings: Settings = read_json(root, "settings.json")?;
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
        let changes = [
            ("accounts.json", serde_json::to_vec_pretty(&accounts)?),
            ("hosts.json", serde_json::to_vec_pretty(&hosts)?),
            ("settings.json", serde_json::to_vec_pretty(&settings)?),
        ];
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
        Ok(
            json!({"ok":true,"hosts":p.hosts.len(),"accounts":p.accounts.len(),"secrets":p.secrets.len(),"keys":p.keys.len()}),
        )
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
            r#"$ErrorActionPreference='Stop';$sid=[Security.Principal.WindowsIdentity]::GetCurrent().User.Value;$n=@(Get-CimInstance Win32_Process -Filter "Name='itda.exe'"|Where-Object {{$_.ProcessId -ne {} -and (Invoke-CimMethod -InputObject $_ -MethodName GetOwnerSid).Sid -eq $sid}}).Count;[Console]::Write($n)"#,
            std::process::id()
        );
        let bytes: Vec<_> = script.encode_utf16().flat_map(u16::to_le_bytes).collect();
        let out = std::process::Command::new("powershell.exe")
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
                if !line.starts_with('#') && !line.starts_with("@revoked") && !line.trim().is_empty() {
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
        return Err(crate::tunnel::classify(&stderr));
    }
    write?;
    String::from_utf8(out).map_err(|_| error("원격 기기의 응답을 읽지 못했습니다"))
}
const MAC_PROBE: &str = r#"set -e
[ "$(uname -s)" = Darwin ] || { echo 'itda-unsupported-platform'; exit 0; }
v=""; for a in /Applications/Itda.app "$HOME/Applications/Itda.app"; do
 if [ -d "$a" ]; then v=$(/usr/libexec/PlistBuddy -c 'Print :CFBundleShortVersionString' "$a/Contents/Info.plist" 2>/dev/null || true); break; fi
done
r=false; pgrep -u "$(id -u)" -x itda >/dev/null 2>&1 && r=true
printf '{"platform":"mac","version":"%s","running":%s}\n' "$v" "$r"
"#;
const WIN_PROBE: &str = r#"
$ErrorActionPreference='Stop'
$sid=[Security.Principal.WindowsIdentity]::GetCurrent().User.Value
$running=@(Get-CimInstance Win32_Process -Filter "Name='itda.exe'" | Where-Object {(Invoke-CimMethod -InputObject $_ -MethodName GetOwnerSid).Sid -eq $sid}).Count -gt 0
$exe=Join-Path $env:LOCALAPPDATA 'Programs\Itda\itda.exe'
$version='';if(Test-Path -LiteralPath $exe){$version=[string](Get-Item -LiteralPath $exe).VersionInfo.ProductVersion}
@{platform='windows';version=$version;running=$running}|ConvertTo-Json -Compress
"#;
fn probe(cfg: &crate::tunnel::SshCfg) -> io::Result<Remote> {
    match run(cfg, "/bin/bash -s", MAC_PROBE.as_bytes()) {
        Ok(out) => {
            if out.lines().any(|l| l == "itda-unsupported-platform") {
                return Err(error("원격 설치는 macOS와 Windows를 지원합니다"));
            }
            if let Some(r) = out
                .lines()
                .find_map(|line| serde_json::from_str::<Remote>(line).ok())
            {
                return Ok(r);
            }
        }
        Err(e) if e.kind() == io::ErrorKind::PermissionDenied => return Err(e),
        Err(_) => {}
    }
    let out = run(cfg, &ps(WIN_PROBE), &[])?;
    out.lines()
        .find_map(|line| serde_json::from_str::<Remote>(line).ok())
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
    Err(error("이 Windows SSH 로그인 세션에서 자격 증명 관리자를 사용할 수 없습니다. 비밀번호 공유를 해제하고, 키 암호가 있는 경우 개인키 공유도 해제해 주세요. 앱과 프로필만 설치할 수 있습니다"))
}

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
    progress("SSH 연결과 원격 기기를 확인하고 있어요…");
    let remote = probe(&cfg)?;
    if crate::update::newer(&remote.version, env!("CARGO_PKG_VERSION")) {
        return Err(error("원격 Itda가 더 최신 버전입니다. 이 기기의 Itda를 먼저 업데이트하세요. 원격 앱과 설정은 변경하지 않았습니다"));
    }
    progress("선택한 설정을 준비하고 있어요…");
    let package = snapshot(&hosts, &accounts, &settings, &choice)?;
    if remote.platform == "windows" && !package.secrets.is_empty() {
        check_remote_vault(&cfg)?;
    }
    let token = random_id()?;
    let summary = json!({"type":"remote_setup","prepared":true,"token":token,"platform":remote.platform,"version":remote.version,"running":remote.running,"address":cfg.host,"user":cfg.user,"port":cfg.port,"hosts":package.hosts.len(),"accounts":package.accounts.len(),"passwords":choice.passwords,"private_keys":choice.private_keys,"key_count":package.keys.len(),"settings":choice.settings,"launch":choice.launch,"close_running":choice.close_running,"fingerprints":fingerprints,"new_host_key":false});
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
    use super::*;
    #[cfg(windows)]
    #[test]
    fn windows_bootstrap_is_valid_powershell_without_executing_it() {
        for command in [win_install("0.9.99", true, true), ps(WIN_VAULT_PROBE)] {
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
