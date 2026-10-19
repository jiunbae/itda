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

pub fn known_hosts() -> PathBuf {
    crate::store::dir().join("known_hosts")
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
fn classify(log: &str) -> io::Error {
    use io::ErrorKind::*;
    let has = |s: &str| log.contains(s);
    if has("REMOTE HOST IDENTIFICATION HAS CHANGED") || has("Host key verification failed") {
        err(PermissionDenied, "SSH 호스트 키가 바뀌었습니다 — 이전에 접속한 Mac과 다른 기기일 수 있습니다")
    } else if has("Permission denied") || has("Too many authentication failures") {
        err(PermissionDenied, "SSH 로그인 실패 — Mac 계정 이름/암호를 확인하세요")
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
        let exe = ssh_exe().ok_or_else(|| err(io::ErrorKind::NotFound, if cfg!(windows) { "ssh.exe 없음 — Windows 'OpenSSH 클라이언트' 기능이 필요합니다" } else { "ssh를 찾을 수 없습니다" }))?;
        // A free local port for the forward (tiny race with other apps; ExitOnForwardFailure catches it).
        let lport = TcpListener::bind("127.0.0.1:0")?.local_addr()?.port();
        let c = &self.cfg;
        let target = if c.user.is_empty() { c.host.clone() } else { format!("{}@{}", c.user, c.host) };
        let me = std::env::current_exe()?;
        let mut cmd = Command::new(exe);
        // A screen tunnel carries input and frames: mark it low-delay. Without a session ssh
        // treats it as bulk, and older OpenSSH (Windows' own) sends bulk as background
        // traffic (DSCP CS1), the lowest Wi-Fi queue.
        cmd.args(["-v", "-N", "-T"])
            .args(["-o", "IPQoS=lowdelay"])
            .args(["-o", "ExitOnForwardFailure=yes"])
            .args(["-o", "StrictHostKeyChecking=accept-new"])
            .arg("-o")
            .arg(format!("UserKnownHostsFile={}", known_hosts().display()))
            .args(["-o", "ServerAliveInterval=10", "-o", "ServerAliveCountMax=3"])
            .args(["-o", "ConnectTimeout=10", "-o", "NumberOfPasswordPrompts=1"])
            .args(["-o", "BatchMode=no"])
            .arg("-p")
            .arg(c.port.to_string())
            .arg("-L")
            .arg(format!("127.0.0.1:{lport}:localhost:{}", c.vnc_port))
            .args(if c.key.is_empty() { vec![] } else { vec!["-i".to_string(), c.key.clone(), "-o".into(), "IdentitiesOnly=yes".into()] })
            .arg(&target)
            .env(ENV_KEYPASS, &c.key_pass)
            .env("SSH_ASKPASS", me)
            .env("SSH_ASKPASS_REQUIRE", "force")
            .env(ENV_SECRET, &c.pass)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::piped());
        quiet(&mut cmd);
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
    let mut cmd = Command::new(keygen);
    cmd.args(["-R", &name, "-f"]).arg(known_hosts()).stdout(Stdio::null()).stderr(Stdio::null());
    let _ = quiet(&mut cmd).status();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn host_split() {
        assert_eq!(split_host("mac.local", 5900), ("mac.local".into(), 5900));
        assert_eq!(split_host("10.0.0.2:5901", 5900), ("10.0.0.2".into(), 5901));
        assert_eq!(split_host("[fe80::1]:22", 5900), ("fe80::1".into(), 22));
        assert_eq!(split_host("fe80::1", 5900), ("fe80::1".into(), 5900));
    }

    #[test]
    fn errors_are_classified() {
        assert_eq!(classify("debug1: x\nuser@h: Permission denied (publickey,password).").kind(), io::ErrorKind::PermissionDenied);
        assert!(classify("ssh: connect to host h port 22: Connection refused").to_string().contains("원격 로그인"));
        assert!(classify("@@@ WARNING: REMOTE HOST IDENTIFICATION HAS CHANGED! @@@").to_string().contains("호스트 키"));
        assert!(classify("ssh: Could not resolve hostname nope: 알려진 호스트가 없습니다.").to_string().contains("찾을 수 없습니다"));
    }
}
