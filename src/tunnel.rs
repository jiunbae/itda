//! Encrypted transport: an SSH local forward through the system OpenSSH client
//! (C:\Windows\System32\OpenSSH\ssh.exe, built into Windows 10 1809+ / 11; /usr/bin/ssh on
//! macOS). Using the OS client keeps crypto out of this binary and honours the user's own
//! keys, agent and ~/.ssh/config. The password is answered through SSH_ASKPASS by
//! this same executable (`itda --askpass`), so nothing is typed into a console.

use std::io::{self, BufRead, BufReader};
use std::net::TcpListener;
#[cfg(windows)]
use std::os::windows::io::AsRawHandle;
#[cfg(windows)]
use std::os::windows::process::CommandExt;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
#[cfg(windows)]
use windows_sys::Win32::System::JobObjects::*;

#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;
const ENV_SECRET: &str = "ITDA_ASKPASS_SECRET";
const ENV_KEYPASS: &str = "ITDA_ASKPASS_KEYPASS";

#[derive(Clone)]
pub struct SshCfg {
    /// Host to SSH into (the Mac), without port.
    pub host: String,
    pub port: u16,
    /// Empty = let ssh decide (config file / current user).
    pub user: String,
    pub pass: String,
    /// Screen Sharing port on the Mac, reached from the Mac itself.
    pub vnc_port: u16,
    /// Private key file (empty = ssh's defaults / agent) and its passphrase.
    pub key: String,
    pub key_pass: String,
}

pub struct Tunnel {
    cfg: SshCfg,
    child: Mutex<Option<Child>>,
    port: Mutex<u16>,
    /// ssh's diagnostic output (it logs to stderr), for error classification.
    log: Arc<Mutex<String>>,
    #[cfg(windows)]
    job: isize,
}

#[cfg(windows)]
pub fn ssh_exe() -> Option<PathBuf> {
    let sys = std::env::var_os("SystemRoot").map(PathBuf::from).unwrap_or_else(|| "C:\\Windows".into());
    let p = sys.join("System32").join("OpenSSH").join("ssh.exe");
    if p.exists() {
        return Some(p);
    }
    std::env::split_paths(&std::env::var_os("PATH")?).map(|d| d.join("ssh.exe")).find(|p| p.exists())
}

#[cfg(not(windows))]
pub fn ssh_exe() -> Option<PathBuf> {
    let p = PathBuf::from("/usr/bin/ssh");
    if p.exists() {
        return Some(p);
    }
    std::env::split_paths(&std::env::var_os("PATH")?).map(|d| d.join("ssh")).find(|p| p.exists())
}

/// Run ssh tools without a console window (Windows); nothing to do elsewhere.
fn quiet(cmd: &mut Command) -> &mut Command {
    #[cfg(windows)]
    cmd.creation_flags(CREATE_NO_WINDOW);
    cmd
}

/// Common SSH authentication for screen tunnels, SFTP and rsync. File workers use their
/// own connection so a bulk transfer cannot block screen/input bytes on one SSH stream.
/// The destination argument: the host alone. The account goes separately as `-l` (see
/// ssh_command), so names with '@' (Microsoft accounts, UPNs: me@outlook.com) work.
pub fn ssh_target(c: &SshCfg) -> io::Result<String> {
    if c.host.is_empty() || c.host.starts_with('-') || c.host.chars().any(|v| v.is_control() || v.is_whitespace() || v == '@') || c.user.starts_with('-') || c.user.chars().any(|v| v.is_control()) {
        return Err(err(io::ErrorKind::InvalidInput, "SSH 기기 주소와 계정 이름을 확인하세요"));
    }
    Ok(c.host.clone())
}

pub fn ssh_command(c: &SshCfg) -> io::Result<Command> {
    ssh_command_with_check(c,false)
}
pub fn ssh_command_strict(c:&SshCfg)->io::Result<Command> {ssh_command_with_check(c,true)}
fn ssh_command_with_check(c:&SshCfg,strict:bool)->io::Result<Command> {
    ssh_target(c)?;
    let exe = ssh_exe().ok_or_else(|| err(io::ErrorKind::NotFound, "OpenSSH 클라이언트를 찾을 수 없습니다"))?;
    let mut cmd = Command::new(exe);
    cmd.arg("-T").args(["-o", if strict{"StrictHostKeyChecking=yes"}else{"StrictHostKeyChecking=accept-new"}, "-o"])
        .arg(known_hosts_option(&known_hosts_files()))
        .args(["-o", "ServerAliveInterval=10", "-o", "ServerAliveCountMax=3", "-o", "ConnectTimeout=10", "-o", "NumberOfPasswordPrompts=1", "-o", "BatchMode=no"])
        .arg("-p").arg(c.port.to_string())
        .args(if c.user.is_empty() { None } else { Some(["-l", c.user.as_str()]) }.into_iter().flatten())
        .env("SSH_ASKPASS", std::env::current_exe()?).env("SSH_ASKPASS_REQUIRE", "force")
        .env(ENV_SECRET, &c.pass).env(ENV_KEYPASS, &c.key_pass);
    if !c.key.is_empty() { cmd.arg("-i").arg(&c.key).args(["-o", "IdentitiesOnly=yes"]); }
    quiet(&mut cmd);
    Ok(cmd)
}

pub fn known_hosts() -> PathBuf {
    crate::store::dir().join("known_hosts")
}

/// The file earlier Itda wrote to by mistake: it passed the path unquoted, and ssh took the
/// first word ("~/Library/Application"). Still read, so host keys, revocations and CAs trusted
/// back then keep their effect. Itda never moves or deletes it; new keys go to the first file,
/// while an explicit "forget changed key" and ssh's own UpdateHostKeys may edit entries in it.
fn legacy_known_hosts() -> Option<PathBuf> {
    let full = known_hosts().to_string_lossy().into_owned();
    let first = full.split_whitespace().next()?;
    (first != full).then(|| PathBuf::from(first)).filter(|p| p.is_file())
}

/// Trust files ssh reads, in order; it writes new host keys to the first one only.
pub fn known_hosts_files() -> Vec<PathBuf> {
    std::iter::once(known_hosts()).chain(legacy_known_hosts()).collect()
}

/// ssh reads this option's value as a whitespace-separated list of files, and macOS's data
/// folder is in "Application Support": each path is quoted.
fn known_hosts_option(files: &[PathBuf]) -> String {
    let quoted: Vec<String> = files.iter().map(|f| format!("\"{}\"", f.to_string_lossy().replace('\\', "\\\\").replace('"', "\\\""))).collect();
    format!("UserKnownHostsFile={}", quoted.join(" "))
}

/// `host[:port]` → (host, port), keeping bracketed IPv6 intact.
pub fn split_host(s: &str, default: u16) -> (String, u16) {
    if let Some(rest) = s.strip_prefix('[') {
        if let Some((h, p)) = rest.split_once(']') {
            return (h.to_string(), p.strip_prefix(':').and_then(|p| p.parse().ok()).unwrap_or(default));
        }
    }
    match s.rsplit_once(':') {
        Some((h, p)) if !h.contains(':') => (h.to_string(), p.parse().unwrap_or(default)),
        _ => (s.to_string(), default),
    }
}

fn err(kind: io::ErrorKind, msg: &str) -> io::Error {
    io::Error::new(kind, msg.to_string())
}

/// Turn ssh's log into the error a person should see.
pub(crate) fn classify(log: &str) -> io::Error {
    use io::ErrorKind::*;
    let has = |s: &str| log.contains(s);
    if has("REMOTE HOST IDENTIFICATION HAS CHANGED") || has("Host key verification failed") {
        err(PermissionDenied, "SSH 호스트 키가 바뀌었습니다 — 이전에 접속한 Mac과 다른 기기일 수 있습니다")
    } else if has("Permission denied") || has("Too many authentication failures") {
        err(PermissionDenied, "SSH 로그인 실패 — 기기 계정 이름/암호를 확인하세요")
    } else if has("Could not resolve hostname") {
        err(NotFound, "호스트를 찾을 수 없습니다")
    } else if has("Connection refused") {
        err(ConnectionRefused, "원격 로그인(SSH)이 꺼져 있습니다")
    } else if has("timed out") {
        err(TimedOut, "SSH 연결 시간 초과")
    } else if has("No route to host") || has("Network is unreachable") {
        err(Other, "네트워크에 도달할 수 없습니다 (os error 10065)")
    } else {
        let last = log.lines().rev().find(|l| !l.starts_with("debug")).unwrap_or("").trim();
        err(Other, &format!("SSH 연결 실패: {last}"))
    }
}

#[cfg(windows)]
fn job() -> isize {
    // Every ssh we start dies with us, even if this process crashes.
    unsafe {
        let j = CreateJobObjectW(std::ptr::null(), std::ptr::null());
        let mut info: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = std::mem::zeroed();
        info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
        SetInformationJobObject(j, JobObjectExtendedLimitInformation, &info as *const _ as _, size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32);
        j as isize
    }
}

/// Keep file transport subprocesses in a kill-on-close job on Windows, including their
/// descendants. macOS transports exit when their pipes close and are reaped on disconnect.
pub fn register_child(child: &Child) {
    #[cfg(windows)] unsafe {
        static FILE_JOB: std::sync::OnceLock<isize> = std::sync::OnceLock::new();
        let handle = FILE_JOB.get_or_init(job);
        AssignProcessToJobObject(*handle as _, child.as_raw_handle() as _);
    }
    #[cfg(not(windows))] let _ = child;
}

impl Tunnel {
    pub fn open(cfg: SshCfg) -> io::Result<Arc<Tunnel>> {
        let t = Arc::new(Tunnel {
            cfg,
            child: Mutex::new(None),
            port: Mutex::new(0),
            log: Arc::new(Mutex::new(String::new())),
            #[cfg(windows)]
            job: job(),
        });
        t.start()?;
        Ok(t)
    }

    fn start(&self) -> io::Result<()> {
        let lport = TcpListener::bind(("127.0.0.1", 0))?.local_addr()?.port();
        let c = &self.cfg;
        let target = ssh_target(c)?;
        let mut cmd = ssh_command(c)?;
        // A screen tunnel carries input and frames: mark it low-delay. Without a session ssh
        // treats it as bulk, and older OpenSSH (Windows' own) sends bulk as background
        // traffic (DSCP CS1), the lowest Wi-Fi queue. (File transfers stay bulk.)
        // Its own connection: through a ControlMaster from the user's ssh config, the forward
        // would be requested of the master and this ssh would exit at once.
        cmd.args(["-o", "ControlMaster=no", "-o", "ControlPath=none"]);
        cmd.args(["-v", "-N", "-o", "IPQoS=lowdelay", "-o", "ExitOnForwardFailure=yes", "-L"])
            .arg(format!("127.0.0.1:{lport}:localhost:{}", c.vnc_port))
            .arg(&target)
            .stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::piped());
        crate::log!("ssh tunnel -> {target}:{} forwarding 127.0.0.1:{lport} -> localhost:{}", c.port, c.vnc_port);
        let mut child = cmd.spawn()?;
        #[cfg(windows)]
        unsafe {
            AssignProcessToJobObject(self.job as _, child.as_raw_handle() as _)
        };
        self.log.lock().unwrap().clear();
        let (log, err_pipe) = (self.log.clone(), child.stderr.take().unwrap());
        std::thread::spawn(move || {
            for line in BufReader::new(err_pipe).lines().map_while(Result::ok) {
                if !line.starts_with("debug1: Reading configuration") && !line.contains("identity file") {
                    crate::log!("ssh: {line}");
                }
                let mut l = log.lock().unwrap();
                l.push_str(&line);
                l.push('\n');
            }
        });
        let end = Instant::now() + Duration::from_secs(30);
        loop {
            if self.log.lock().unwrap().contains("Local forwarding listening on") {
                break;
            }
            if let Some(st) = child.try_wait()? {
                std::thread::sleep(Duration::from_millis(100)); // let the reader drain
                let e = classify(&self.log.lock().unwrap());
                crate::log!("ssh exited ({st}): {e}");
                return Err(e);
            }
            if Instant::now() > end {
                let _ = child.kill();
                return Err(err(io::ErrorKind::TimedOut, "SSH 연결 시간 초과"));
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        *self.port.lock().unwrap() = lport;
        *self.child.lock().unwrap() = Some(child);
        Ok(())
    }

    /// Where to connect for RFB; restarts ssh if it died (network change, sleep).
    pub fn addr(&self) -> io::Result<String> {
        let alive = self.child.lock().unwrap().as_mut().is_some_and(|c| matches!(c.try_wait(), Ok(None)));
        if !alive {
            crate::log!("ssh tunnel down, restarting");
            self.start()?;
        }
        Ok(format!("127.0.0.1:{}", self.port.lock().unwrap()))
    }

    /// The Mac accepted SSH but refused the forwarded connection: Screen Sharing is off.
    pub fn vnc_refused(&self) -> bool {
        self.log.lock().unwrap().contains("open failed")
    }
}

impl Drop for Tunnel {
    fn drop(&mut self) {
        if let Some(mut c) = self.child.lock().unwrap().take() {
            let _ = c.kill();
        }
    }
}

/// `itda --askpass <prompt>`: ssh asks for a secret; answer from the environment.
pub fn askpass(prompt: &str) -> i32 {
    let p = prompt.to_ascii_lowercase();
    if p.contains("yes/no") {
        return 1; // never auto-accept anything
    }
    let secret = std::env::var(if p.contains("passphrase") { ENV_KEYPASS } else { ENV_SECRET }).unwrap_or_default();
    if secret.is_empty() {
        return 1;
    }
    println!("{secret}");
    0
}

/// Forget the stored host key for `host:port` (after the user confirmed it really changed).
pub fn forget_host_key(host: &str, port: u16) {
    let Some(ssh) = ssh_exe() else { return };
    let keygen = ssh.with_file_name(if cfg!(windows) { "ssh-keygen.exe" } else { "ssh-keygen" });
    let name = if port == 22 { host.to_string() } else { format!("[{host}]:{port}") };
    for file in known_hosts_files().into_iter().filter(|f| f.is_file()) {
        let mut cmd = Command::new(&keygen);
        cmd.args(["-R", &name, "-f"]).arg(file).stdout(Stdio::null()).stderr(Stdio::null());
        let _ = quiet(&mut cmd).status();
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn known_hosts_paths_with_spaces_stay_whole() {
        let mac = PathBuf::from("/Users/a/Library/Application Support/Itda/known_hosts");
        assert_eq!(known_hosts_option(&[mac.clone()]), "UserKnownHostsFile=\"/Users/a/Library/Application Support/Itda/known_hosts\"");
        assert_eq!(known_hosts_option(&[mac, PathBuf::from("/Users/a/Library/Application")]), "UserKnownHostsFile=\"/Users/a/Library/Application Support/Itda/known_hosts\" \"/Users/a/Library/Application\"");
        assert_eq!(known_hosts_option(&[PathBuf::from(r"C:\Users\A B\AppData\Roaming\Itda\known_hosts")]), r#"UserKnownHostsFile="C:\\Users\\A B\\AppData\\Roaming\\Itda\\known_hosts""#);
    }

    use super::*;

    #[test]
    fn host_split() {
        assert_eq!(split_host("mac.local", 5900), ("mac.local".into(), 5900));
        assert_eq!(split_host("10.0.0.2:5901", 5900), ("10.0.0.2".into(), 5901));
        assert_eq!(split_host("[fe80::1]:22", 5900), ("fe80::1".into(), 22));
        assert_eq!(split_host("fe80::1", 5900), ("fe80::1".into(), 5900));
    }

    fn cfg(user: &str, host: &str) -> SshCfg {
        SshCfg { host: host.into(), port: 22, user: user.into(), pass: String::new(), vnc_port: 5900, key: String::new(), key_pass: String::new() }
    }

    /// The account is its own argument: '@' in it (me@outlook.com) is fine, and the
    /// destination is the host alone, as it was before the file-transfer refactor accepted it.
    #[test]
    fn account_with_at_sign() {
        let c = cfg("me@outlook.com", "pc.local");
        assert_eq!(ssh_target(&c).unwrap(), "pc.local");
        if let Ok(cmd) = ssh_command(&c) {
            let args: Vec<String> = cmd.get_args().map(|a| a.to_string_lossy().into_owned()).collect();
            let at = args.iter().position(|a| a == "-l").expect("-l");
            assert_eq!(args[at + 1], "me@outlook.com");
            assert!(!args.iter().any(|a| a.contains("@pc.local")));
        }
        assert!(ssh_command(&cfg("", "pc.local")).map_or(true, |c| !c.get_args().any(|a| a == "-l")));
        for (u, h) in [("-oProxyCommand=x", "pc"), ("me", "-oProxyCommand=x"), ("me", "a@b"), ("me", "pc host"), ("me\n", "pc"), ("me", "")] {
            assert!(ssh_target(&cfg(u, h)).is_err(), "{u:?} {h:?}");
        }
    }

    #[test]
    fn errors_are_classified() {
        assert_eq!(classify("debug1: x\nuser@h: Permission denied (publickey,password).").kind(), io::ErrorKind::PermissionDenied);
        assert!(classify("ssh: connect to host h port 22: Connection refused").to_string().contains("원격 로그인"));
        assert!(classify("@@@ WARNING: REMOTE HOST IDENTIFICATION HAS CHANGED! @@@").to_string().contains("호스트 키"));
        assert!(classify("ssh: Could not resolve hostname nope: 알려진 호스트가 없습니다.").to_string().contains("찾을 수 없습니다"));
    }
}
