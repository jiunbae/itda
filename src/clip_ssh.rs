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
    stop: Arc<AtomicBool>,
    watcher: Arc<Mutex<Option<Child>>>,
    /// The last text that went either way (an echo of it is not passed on again).
    last: Arc<Mutex<String>>,
    /// Text to put on that Mac, newest only (one sender works through it, in order), and the
    /// pbcopy running for it.
    outbox: Arc<(Mutex<Option<String>>, std::sync::Condvar)>,
    sending: Arc<Mutex<Option<Child>>>,
}

impl Drop for Bridge {
    fn drop(&mut self) {
        {
            // (Under the sender's lock: it can't miss the wake-up between its check and wait.)
            let _pending = self.outbox.0.lock().unwrap();
            self.stop.store(true, Ordering::Relaxed);
            self.outbox.1.notify_all();
        }
        for child in [&self.watcher, &self.sending] {
            reap(child);
        }
    }
}

/// End a child process kept in `slot`, if any.
fn reap(slot: &Mutex<Option<Child>>) {
    if let Some(mut c) = slot.lock().unwrap().take() {
        let _ = c.kill();
        let _ = c.wait();
    }
}

/// Ends the child kept in a slot whenever the function using it returns (an error included).
struct Reaping<'a>(&'a Mutex<Option<Child>>);
impl Drop for Reaping<'_> {
    fn drop(&mut self) {
        reap(self.0);
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
    // (A password typed for this connection and not saved counts too, as for the screen.)
    let pass = crate::gateway::typed(id).or_else(|| login.pass.clone()).unwrap_or_default();
    Some(crate::hp_upgrade::ssh_cfg_with(&host, &login, pass))
}

/// Start the clipboard bridge for connection `id`, if it is such a Mac. `copied` gets text
/// copied on that Mac (on another thread).
pub fn start(id: &str, copied: impl Fn(String) + Send + 'static) -> Option<Bridge> {
    Some(start_with(target(id)?, copied))
}

fn start_with(cfg: SshCfg, copied: impl Fn(String) + Send + 'static) -> Bridge {
    let bridge = Bridge { stop: Arc::default(), watcher: Arc::default(), last: Arc::default(), outbox: Arc::default(), sending: Arc::default() };
    let (stop, outbox, sending) = (bridge.stop.clone(), bridge.outbox.clone(), bridge.sending.clone());
    let sender_cfg = cfg.clone();
    std::thread::spawn(move || {
        // One sender, the newest text only: an older copy can't land after a newer one.
        loop {
            let text = {
                let mut pending = outbox.0.lock().unwrap();
                while pending.is_none() && !stop.load(Ordering::Relaxed) {
                    pending = outbox.1.wait(pending).unwrap();
                }
                if stop.load(Ordering::Relaxed) {
                    return;
                }
                pending.take().unwrap_or_default()
            };
            if let Err(e) = put(&sender_cfg, &text, &stop, &sending) {
                crate::log!("clipboard over SSH: couldn't copy to the Mac: {e}");
            }
        }
    });
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
    let _reaping = Reaping(watcher);
    let mut cmd = crate::tunnel::ssh_command(cfg)?;
    cmd.arg(crate::tunnel::ssh_target(cfg)?).arg("/bin/bash -s");
    let mut child = cmd.stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::null()).spawn()?;
    let input = child.stdin.take();
    let out = child.stdout.take();
    // Kept where the bridge (or this function's return) ends it, before anything can fail.
    *watcher.lock().unwrap() = Some(child);
    if stop.load(Ordering::Relaxed) {
        return Ok(());
    }
    input.ok_or_else(|| std::io::Error::other("no stdin"))?.write_all(WATCH.as_bytes())?;
    let out = out.ok_or_else(|| std::io::Error::other("no stdout"))?;
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
    Ok(())
}

/// Put `text` on that Mac's clipboard (pbcopy over SSH), stoppable through `sending`.
fn put(cfg: &SshCfg, text: &str, stop: &AtomicBool, sending: &Mutex<Option<Child>>) -> std::io::Result<()> {
    let _reaping = Reaping(sending);
    let mut cmd = crate::tunnel::ssh_command(cfg)?;
    cmd.arg(crate::tunnel::ssh_target(cfg)?).arg("LANG=en_US.UTF-8 pbcopy");
    let mut child = cmd.stdin(Stdio::piped()).stdout(Stdio::null()).stderr(Stdio::null()).spawn()?;
    let input = child.stdin.take();
    *sending.lock().unwrap() = Some(child);
    if stop.load(Ordering::Relaxed) {
        return Ok(());
    }
    {
        let mut input = input.ok_or_else(|| std::io::Error::other("no stdin"))?;
        input.write_all(text.as_bytes())?;
    } // (closed: pbcopy reads to the end)
    // Wait without holding the slot, so the bridge can still end it.
    loop {
        if let Some(c) = sending.lock().unwrap().as_mut() {
            if let Some(status) = c.try_wait()? {
                return if status.success() { Ok(()) } else { Err(std::io::Error::other("pbcopy failed")) };
            }
        } else {
            return Ok(()); // ended by the bridge
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
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
        *self.outbox.0.lock().unwrap() = Some(text.to_string());
        self.outbox.1.notify_all();
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
