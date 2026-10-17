//! Session process: one viewer window per connection. The home window launches
//! `itda --session`, writes one JSON `Launch` line to stdin and reads JSON events
//! (one per line) from stdout. Run without a home (command line), events go nowhere.

use crate::rfb::{self, Creds};
use crate::store::{HostOpts, Ssh};
use crate::tunnel::{self, SshCfg, Tunnel};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::io::{self, BufRead, Write};
use std::sync::atomic::{AtomicBool, Ordering::Relaxed};
use std::sync::Arc;

#[derive(Serialize, Deserialize, Clone, Default)]
#[serde(default)]
pub struct Launch {
    pub id: String,
    pub title: String,
    pub host: String,
    pub user: String,
    pub pass: String,
    pub opts: HostOpts,
    /// Reach Screen Sharing through an SSH tunnel to the Mac.
    pub ssh: Ssh,
    /// SSH private key from the account (empty = defaults) and its passphrase.
    pub ssh_key: String,
    pub key_pass: String,
    /// "ctrl_alt" | "ctrl_shift"
    pub hotkey: String,
    /// "auto" | "pinned" | "hidden"
    pub toolbar: String,
    pub thumbnails: bool,
}

static MANAGED: AtomicBool = AtomicBool::new(false);

pub fn managed() -> bool {
    MANAGED.load(Relaxed)
}

/// Send an event to the home process (no-op when run standalone).
pub fn emit(v: Value) {
    if managed() {
        let mut o = std::io::stdout().lock();
        let _ = writeln!(o, "{v}");
        let _ = o.flush();
    }
}

pub fn run_managed() {
    MANAGED.store(true, Relaxed);
    let mut line = String::new();
    let _ = std::io::stdin().lock().read_line(&mut line);
    let Ok(l) = serde_json::from_str::<Launch>(&line) else {
        crate::log!("session: bad launch line");
        return;
    };
    let _ = rfb::PROGRESS.set(|stage| emit(json!({"ev": "stage", "stage": stage})));
    crate::log!("session {} -> {}{}", l.id, l.host, if l.ssh.on { " (ssh)" } else { "" });
    run(l);
}

fn progress(stage: &str) {
    if let Some(f) = rfb::PROGRESS.get() {
        f(stage);
    }
}

/// Connect and, on success, open the viewer window (blocks until it closes).
pub fn run(l: Launch) {
    let creds = Creds { user: l.user.clone(), pass: l.pass.clone() };
    let mut tunnel_ref: Option<Arc<Tunnel>> = None;
    let r = (|| -> io::Result<_> {
        if !l.ssh.on {
            let addr = rfb::addr(&l.host);
            let sess = rfb::connect(&addr, &creds)?;
            let dial: crate::net::Dial = Arc::new(move || Ok(addr.clone()));
            return Ok((sess, dial, rfb::addr(&l.host), false));
        }
        progress("ssh");
        let (host, vnc_port) = tunnel::split_host(&l.host, 5900);
        let cfg = SshCfg {
            host: host.clone(),
            port: if l.ssh.port == 0 { 22 } else { l.ssh.port },
            user: if l.ssh.user.is_empty() { l.user.clone() } else { l.ssh.user.clone() },
            // The Mac account password works for both Remote Login and Screen Sharing.
            pass: l.pass.clone(),
            vnc_port,
            key: l.ssh_key.clone(),
            key_pass: l.key_pass.clone(),
        };
        let shown = format!("{}@{host} (SSH)", cfg.user);
        let t = Tunnel::open(cfg)?;
        tunnel_ref = Some(t.clone());
        let sess = rfb::connect(&t.addr()?, &creds).map_err(|e| {
            if t.vnc_refused() {
                io::Error::new(io::ErrorKind::ConnectionRefused, "SSH는 연결됐지만 화면 공유가 꺼져 있습니다")
            } else {
                e
            }
        })?;
        let dial: crate::net::Dial = Arc::new(move || t.addr());
        Ok((sess, dial, shown, true))
    })();
    match r {
        Ok((sess, dial, addr, secure)) => crate::ui::run(crate::ui::Conn { sess, addr, dial, secure, creds }, l),
        Err(e) => {
            let kind = rfb::classify(&e);
            crate::log!("connect failed: {e} [{kind}]");
            if managed() {
                emit(json!({"ev": "error", "kind": kind, "msg": e.to_string()}));
            } else {
                crate::ui::message(&format!("연결할 수 없습니다: {e}\n\n로그: {}", crate::log::path().display()), true);
            }
        }
    }
    drop(tunnel_ref);
}
