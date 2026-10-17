//! Tailscale integration through its CLI: list the tailnet's devices (so Macs anywhere
//! show up next to the LAN ones) and publish the web viewer with `tailscale serve`.

use serde::Serialize;
use serde_json::Value;
use std::os::windows::process::CommandExt;
use std::path::PathBuf;
use std::process::{Command, Stdio};

const CREATE_NO_WINDOW: u32 = 0x0800_0000;

fn exe() -> Option<PathBuf> {
    let pf = std::env::var_os("ProgramFiles").map(PathBuf::from).unwrap_or_else(|| "C:\\Program Files".into());
    let p = pf.join("Tailscale").join("tailscale.exe");
    p.exists().then_some(p)
}

fn run(args: &[&str]) -> Option<(bool, String)> {
    let out = Command::new(exe()?)
        .args(args)
        .stdin(Stdio::null())
        .creation_flags(CREATE_NO_WINDOW)
        .output()
        .ok()?;
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
    peers.sort_by(|a, b| (!a.online, &a.name).cmp(&(!b.online, &b.name)));
    Status {
        installed: true,
        running: v["BackendState"].as_str() == Some("Running"),
        me: peer(&v["Self"]),
        peers,
        https: v["CertDomains"].as_array().is_some_and(|a| !a.is_empty()),
    }
}

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
