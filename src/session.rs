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
    /// Mac wheel speed and acceleration (see store::Settings).
    pub mac_scroll: String,
    pub scroll_accel: bool,
    pub thumbnails: bool,
    /// Itda's high-performance sharing on this Mac, tried first (else Screen Sharing).
    pub hp: Option<HpRoute>,
}

/// How to reach a Mac's high-performance sharing: SSH to it, then its loopback port.
#[derive(Serialize, Deserialize, Clone, Default)]
#[serde(default)]
pub struct HpRoute {
    pub ssh_host: String,
    pub ssh_port: u16,
    pub ssh_user: String,
    /// The Mac account's password for SSH (empty: this connection's).
    pub ssh_pass: String,
    pub key_alias: String,
    /// SSH private key and passphrase of the account it signs in with (empty: this one's).
    pub key: String,
    pub key_pass: String,
    /// Its port on the Mac, and the sharing password.
    pub port: u16,
    pub pass: String,
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

/// Itda's high-performance sharing on a Mac, through SSH. Nothing answering there (Itda quit,
/// or the Mac restarted): start it in the background and try once more.
fn open_hp(l: &Launch, hp: &HpRoute) -> io::Result<(rfb::Session, Arc<Tunnel>, Creds)> {
    progress("ssh");
    let cfg = SshCfg {
        host: hp.ssh_host.clone(),
        port: if hp.ssh_port == 0 { 22 } else { hp.ssh_port },
        user: if hp.ssh_user.is_empty() { l.user.clone() } else { hp.ssh_user.clone() },
        pass: if hp.ssh_pass.is_empty() { l.pass.clone() } else { hp.ssh_pass.clone() },
        vnc_port: if hp.port == 0 { crate::host_mac_port() } else { hp.port },
        key: if hp.key.is_empty() { l.ssh_key.clone() } else { hp.key.clone() },
        key_pass: if hp.key.is_empty() { l.key_pass.clone() } else { hp.key_pass.clone() },
        key_alias: hp.key_alias.clone(),
    };
    let t = Tunnel::open(cfg.clone())?;
    let creds = Creds { user: String::new(), pass: hp.pass.clone() };
    let quiet = |e: &io::Error| matches!(e.kind(), io::ErrorKind::UnexpectedEof | io::ErrorKind::ConnectionRefused | io::ErrorKind::ConnectionReset | io::ErrorKind::ConnectionAborted);
    let mut sess = rfb::connect(&t.addr()?, &creds);
    if sess.as_ref().is_err_and(quiet) && crate::hp_upgrade::start_sharing(&cfg).is_ok() {
        sess = rfb::connect(&t.addr()?, &creds);
    }
    Ok((sess?, t, creds))
}

/// Connect and, on success, open the viewer window (blocks until it closes).
pub fn run(l: Launch) {
    let mut creds = Creds { user: l.user.clone(), pass: l.pass.clone() };
    let mut tunnel_ref: Option<Arc<Tunnel>> = None;
    let r = (|| -> io::Result<_> {
        // High-performance sharing first: a fraction of Screen Sharing's traffic for a moving
        // screen. Anything in the way (SSH, Itda not running there, a locked screen) falls
        // back to Screen Sharing.
        if let Some(hp) = &l.hp {
            match open_hp(&l, hp) {
                Ok((sess, t, hp_creds)) => {
                    crate::log!("high-performance sharing on {}", hp.ssh_host);
                    tunnel_ref = Some(t.clone());
                    creds = hp_creds;
                    let shown = format!("{} (Itda)", hp.ssh_host);
                    let dial: crate::net::Dial = Arc::new(move || t.addr());
                    return Ok((sess, dial, shown, true));
                }
                Err(e) => crate::log!("high-performance sharing on {} unavailable, using Screen Sharing: {e}", hp.ssh_host),
            }
        }
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
            key_alias: String::new(),
        };
        let shown = format!("{}@{host} (SSH)", cfg.user);
        let t = Tunnel::open(cfg)?;
        tunnel_ref = Some(t.clone());
        let sess = rfb::connect(&t.addr()?, &creds).map_err(|e| {
            if t.vnc_refused() {
                rfb::kinded(io::ErrorKind::ConnectionRefused, "refused", tr!("conn.sshNoScreenSharing"))
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
                crate::ui::message(&tr!("conn.failedLog", error = e, log = crate::log::path().display()), true);
            }
        }
    }
    drop(tunnel_ref);
}
