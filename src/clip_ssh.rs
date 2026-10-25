//! The clipboard with a Mac over SSH, beside its screen: macOS Screen Sharing doesn't exchange
//! the clipboard with other VNC viewers, so without Itda's high-performance sharing copying
//! between the two Macs did nothing. A connection with SSH (its tunnel, or high-performance
//! setup) gets a small watcher on that Mac: text copied there comes here, and text offered to
//! the connection goes there with `pbcopy`.

use crate::store::{self, Host};
use crate::tunnel::SshCfg;
use std::io::{BufRead, BufReader, Write};
use std::process::{Child, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

/// Up to this much text either way (the viewer's own limit).
const MAX: usize = 256 << 10;

/// The remote side: every second, if the clipboard changed, one line "C <base64 of the text>".
/// The first line is the clipboard as found (not copied here: connecting mustn't overwrite
/// what's on this Mac's clipboard).
const WATCH: &str = r#"export LANG=en_US.UTF-8
last=""
while :; do
  h=$(pbpaste 2>/dev/null | shasum | cut -c1-40)
  if [ "$h" != "$last" ]; then last=$h; printf 'C '; pbpaste 2>/dev/null | head -c 262144 | base64 | tr -d '\n'; echo; fi
  sleep 1
done
"#;

pub struct Bridge {
    cfg: SshCfg,
    stop: Arc<AtomicBool>,
    watcher: Arc<Mutex<Option<Child>>>,
    /// The last text that went either way (an echo of it is not passed on again).
    last: Arc<Mutex<String>>,
}

impl Drop for Bridge {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(mut c) = self.watcher.lock().unwrap().take() {
            let _ = c.kill();
            let _ = c.wait();
        }
    }
}

/// Whether connection `id` is a Mac this app reaches with SSH (its screen tunnel or
/// high-performance sharing), and with what.
fn target(id: &str) -> Option<SshCfg> {
    let host: Host = store::load_hosts().into_iter().find(|h| h.id == id)?;
    let mac = matches!(host.os.as_str(), "mac" | "macos" | "") && !host.is_rdp();
    if !mac || !(host.ssh.on || host.hp.on) {
        return None;
    }
    let login = store::resolve(&host, &store::load_accounts());
    let pass = login.pass.clone().unwrap_or_default();
    Some(crate::hp_upgrade::ssh_cfg_with(&host, &login, pass))
}

/// Start the clipboard bridge for connection `id`, if it is such a Mac. `copied` gets text
/// copied on that Mac (on another thread).
pub fn start(id: &str, copied: impl Fn(String) + Send + 'static) -> Option<Bridge> {
    Some(start_with(target(id)?, copied))
}

fn start_with(cfg: SshCfg, copied: impl Fn(String) + Send + 'static) -> Bridge {
    let bridge = Bridge { cfg: cfg.clone(), stop: Arc::default(), watcher: Arc::default(), last: Arc::default() };
    let (stop, watcher, last) = (bridge.stop.clone(), bridge.watcher.clone(), bridge.last.clone());
    std::thread::spawn(move || {
        // Restarted after a dropped SSH connection, a few seconds apart, until the bridge goes.
        while !stop.load(Ordering::Relaxed) {
            if let Err(e) = watch(&cfg, &stop, &watcher, &last, &copied) {
                crate::log!("clipboard over SSH: {e}");
            }
            for _ in 0..50 {
                if stop.load(Ordering::Relaxed) {
                    return;
                }
                std::thread::sleep(std::time::Duration::from_millis(100));
            }
        }
    });
    bridge
}

fn watch(cfg: &SshCfg, stop: &AtomicBool, watcher: &Mutex<Option<Child>>, last: &Mutex<String>, copied: &dyn Fn(String)) -> std::io::Result<()> {
    let mut cmd = crate::tunnel::ssh_command(cfg)?;
    cmd.arg(crate::tunnel::ssh_target(cfg)?).arg("/bin/bash -s");
    let mut child = cmd.stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::null()).spawn()?;
    child.stdin.take().ok_or_else(|| std::io::Error::other("no stdin"))?.write_all(WATCH.as_bytes())?;
    let out = child.stdout.take().ok_or_else(|| std::io::Error::other("no stdout"))?;
    *watcher.lock().unwrap() = Some(child);
    let mut first = true;
    for line in BufReader::new(out).lines() {
        if stop.load(Ordering::Relaxed) {
            break;
        }
        let line = line?;
        let Some(encoded) = line.strip_prefix("C ") else { continue };
        let text = decode(encoded);
        if std::mem::take(&mut first) {
            *last.lock().unwrap() = text;
            continue;
        }
        let mut seen = last.lock().unwrap();
        if text.is_empty() || text.len() > MAX || *seen == text {
            continue;
        }
        *seen = text.clone();
        drop(seen);
        copied(text);
    }
    if let Some(mut c) = watcher.lock().unwrap().take() {
        let _ = c.kill();
        let _ = c.wait();
    }
    Ok(())
}

impl Bridge {
    /// Put `text` on that Mac's clipboard (in the background).
    pub fn send(&self, text: &str) {
        if text.is_empty() || text.len() > MAX {
            return;
        }
        {
            let mut seen = self.last.lock().unwrap();
            if *seen == text {
                return;
            }
            *seen = text.to_string();
        }
        let (cfg, text) = (self.cfg.clone(), text.to_string());
        std::thread::spawn(move || {
            let result = (|| -> std::io::Result<()> {
                let mut cmd = crate::tunnel::ssh_command(&cfg)?;
                cmd.arg(crate::tunnel::ssh_target(&cfg)?).arg("LANG=en_US.UTF-8 pbcopy");
                let mut child = cmd.stdin(Stdio::piped()).stdout(Stdio::null()).stderr(Stdio::null()).spawn()?;
                child.stdin.take().ok_or_else(|| std::io::Error::other("no stdin"))?.write_all(text.as_bytes())?;
                if !child.wait()?.success() {
                    return Err(std::io::Error::other("pbcopy failed"));
                }
                Ok(())
            })();
            if let Err(e) = result {
                crate::log!("clipboard over SSH: couldn't copy to the Mac: {e}");
            }
        });
    }
}

/// The text of a base64 line (empty if it isn't valid UTF-8 base64).
fn decode(encoded: &str) -> String {
    crate::ws::base64_decode(encoded).and_then(|b| String::from_utf8(b).ok()).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    /// Live check against a real Mac over SSH (its clipboard is saved and put back):
    /// ITDA_CLIP_SSH_HOST=jiun-mbp cargo test --release clip_bridge_live -- --ignored --nocapture
    #[test]
    #[ignore]
    fn clip_bridge_live() {
        use std::process::Command;
        let host = std::env::var("ITDA_CLIP_SSH_HOST").expect("ITDA_CLIP_SSH_HOST");
        let ssh = |cmd: &str, input: Option<&str>| {
            let mut c = Command::new("ssh").args([host.as_str(), cmd]).stdin(std::process::Stdio::piped()).stdout(std::process::Stdio::piped()).spawn().unwrap();
            if let Some(i) = input { std::io::Write::write_all(&mut c.stdin.take().unwrap(), i.as_bytes()).unwrap(); } else { drop(c.stdin.take()); }
            String::from_utf8(c.wait_with_output().unwrap().stdout).unwrap()
        };
        let saved = ssh("pbpaste", None);
        let cfg = super::SshCfg { host: host.clone(), port: 22, user: String::new(), pass: String::new(), vnc_port: 5900, key: String::new(), key_pass: String::new(), key_alias: String::new() };
        let (tx, rx) = std::sync::mpsc::channel();
        let bridge = super::start_with(cfg, move |t| { let _ = tx.send(t); });
        std::thread::sleep(std::time::Duration::from_secs(4)); // the watcher's first look
        ssh("LANG=en_US.UTF-8 pbcopy", Some("원격 → 이 Mac ✅ 브리지"));
        let got = rx.recv_timeout(std::time::Duration::from_secs(10)).expect("nothing came from the Mac");
        assert_eq!(got, "원격 → 이 Mac ✅ 브리지");
        bridge.send("이 Mac → 원격 🌍 브리지");
        std::thread::sleep(std::time::Duration::from_secs(4));
        let there = ssh("LANG=en_US.UTF-8 pbpaste", None);
        // (No echo of our own text comes back.)
        assert!(rx.recv_timeout(std::time::Duration::from_secs(3)).is_err(), "own text echoed back");
        drop(bridge);
        ssh("LANG=en_US.UTF-8 pbcopy", Some(&saved));
        assert_eq!(there, "이 Mac → 원격 🌍 브리지");
    }

    #[test]
    fn decodes_korean_lines() {
        let line = crate::ws::base64("클립보드 🌍\n둘째".as_bytes());
        assert_eq!(super::decode(&line), "클립보드 🌍\n둘째");
        assert_eq!(super::decode("@@"), "");
    }
}
