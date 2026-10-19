//! Tailscale integration through its CLI: list the tailnet's devices (so Macs anywhere
//! show up next to the LAN ones) and publish the web viewer with `tailscale serve`.

use serde::Serialize;
use serde_json::Value;
#[cfg(windows)]
use std::os::windows::process::CommandExt;
use std::path::PathBuf;
use std::process::{Command, Stdio};

#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

#[cfg(windows)]
fn exe() -> Option<PathBuf> {
    let pf = std::env::var_os("ProgramFiles").map(PathBuf::from).unwrap_or_else(|| "C:\\Program Files".into());
    let p = pf.join("Tailscale").join("tailscale.exe");
    p.exists().then_some(p)
}

/// The Mac App Store / standalone app ships its CLI inside the bundle; Homebrew puts it on PATH.
#[cfg(not(windows))]
fn exe() -> Option<PathBuf> {
    let app = PathBuf::from("/Applications/Tailscale.app/Contents/MacOS/Tailscale");
    if app.exists() {
        return Some(app);
    }
    ["/opt/homebrew/bin/tailscale", "/usr/local/bin/tailscale"].iter().map(PathBuf::from).find(|p| p.exists())
}

fn run(args: &[&str]) -> Option<(bool, String)> {
    let mut cmd = Command::new(exe()?);
    cmd.args(args).stdin(Stdio::null());
    // The app bundle's binary is GUI and CLI in one; launched outside a terminal (Itda opened
    // from Finder) it tries to start the GUI and never answers unless told to be the CLI.
    #[cfg(not(windows))]
    cmd.env("TAILSCALE_BE_CLI", "1");
    #[cfg(windows)]
    cmd.creation_flags(CREATE_NO_WINDOW);
    let out = cmd.output().ok()?;
    let text = format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
    Some((out.status.success(), text))
}

#[derive(Serialize, Clone, Debug, Default)]
pub struct Peer {
    pub name: String,
    /// MagicDNS name without the trailing dot ("mac-mini.tail1234.ts.net").
    pub dns: String,
    pub ip: String,
    /// "macOS" | "windows" | "linux" | "iOS" | …
    pub os: String,
    pub online: bool,
}

#[derive(Serialize, Clone, Debug, Default)]
pub struct Status {
    pub installed: bool,
    pub running: bool,
    pub me: Peer,
    pub peers: Vec<Peer>,
    /// HTTPS certificates are enabled for the tailnet (needed by `tailscale serve`).
    pub https: bool,
}

fn peer(v: &Value) -> Peer {
    Peer {
        name: v["HostName"].as_str().unwrap_or("").to_string(),
        dns: v["DNSName"].as_str().unwrap_or("").trim_end_matches('.').to_string(),
        ip: v["TailscaleIPs"].get(0).and_then(Value::as_str).unwrap_or("").to_string(),
        os: v["OS"].as_str().unwrap_or("").to_string(),
        online: v["Online"].as_bool().unwrap_or(false),
    }
}

pub fn status() -> Status {
    let Some((_, out)) = run(&["status", "--json"]) else { return Status::default() };
    let Ok(v) = serde_json::from_str::<Value>(&out) else { return Status { installed: true, ..Status::default() } };
    let mut peers: Vec<Peer> = v["Peer"].as_object().map(|m| m.values().map(peer).collect()).unwrap_or_default();
    keep_openable(&mut peers);
    peers.sort_by(|a, b| (!a.online, &a.name).cmp(&(!b.online, &b.name)));
    Status {
        installed: true,
        running: v["BackendState"].as_str() == Some("Running"),
        me: peer(&v["Self"]),
        peers,
        https: v["CertDomains"].as_array().is_some_and(|a| !a.is_empty()),
    }
}

/// The ports a device of this kind is opened on: screen sharing / VNC, and for a PC also
/// Remote Desktop. (Linux VNC servers often sit on display :1.)
fn ports(os: &str) -> &'static [u16] {
    match os {
        "macOS" => &[5900],
        "windows" => &[5900, 3389],
        "linux" => &[5900, 5901],
        _ => &[],
    }
}

/// Drop online devices with nothing to open: none of their ports answers. Offline ones
/// stay (they can't be asked; they'll answer once they're back). All asked at once.
fn keep_openable(peers: &mut Vec<Peer>) {
    let open: Vec<bool> = std::thread::scope(|sc| {
        let asks: Vec<_> = peers
            .iter()
            .map(|p| {
                sc.spawn(move || {
                    let Ok(ip) = p.ip.parse::<std::net::IpAddr>() else { return false };
                    !p.online || ports(&p.os).iter().any(|&port| std::net::TcpStream::connect_timeout(&(ip, port).into(), PROBE).is_ok())
                })
            })
            .collect();
        asks.into_iter().map(|a| a.join().unwrap_or(false)).collect()
    });
    let mut i = 0;
    peers.retain(|_| {
        i += 1;
        open[i - 1]
    });
}

const PROBE: std::time::Duration = std::time::Duration::from_millis(1000);

/// Is the web viewer currently published over HTTPS on this tailnet?
pub fn serving(port: u16) -> bool {
    run(&["serve", "status", "--json"]).is_some_and(|(_, out)| out.contains(&format!("127.0.0.1:{port}")))
}

/// `tailscale serve`: https://<this-pc>.<tailnet>.ts.net/ → the local web viewer.
pub fn serve(port: u16, on: bool) -> Result<(), String> {
    let target = format!("http://127.0.0.1:{port}");
    let args: Vec<&str> = if on { vec!["serve", "--bg", "--yes", &target] } else { vec!["serve", "--https=443", "off"] };
    match run(&args) {
        None => Err("Tailscale이 설치되어 있지 않습니다".into()),
        Some((true, _)) => Ok(()),
        Some((false, out)) => {
            crate::log!("tailscale serve: {out}");
            let line = out.lines().find(|l| !l.trim().is_empty()).unwrap_or("").trim();
            Err(if out.contains("Access denied") || out.contains("operator") {
                "권한이 없습니다 — 관리자 PowerShell에서 'tailscale set --operator=%USERNAME%'을 한 번 실행하세요".into()
            } else if out.contains("HTTPS") || out.contains("cert") {
                "Tailscale 관리 콘솔(DNS)에서 HTTPS 인증서를 먼저 켜세요".into()
            } else {
                line.to_string()
            })
        }
    }
}
