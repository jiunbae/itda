//! "고성능 모드": from a normal (Apple Screen Sharing) connection to a Mac, set up Itda's own
//! high-performance sharing on that Mac over SSH, so the session can move to it.
//!
//! On the Mac, as the person logged in at its screen: install (or update) Itda from the
//! release matching this one (checked against the release's SHA256SUMS) or from a given app
//! bundle, turn high-performance sharing on (a marker in Itda's data folder), (re)start Itda
//! hidden, and wait until it listens. The macOS permission prompts it raises appear on that
//! Mac's screen, which the person sees and answers through the connection they already have.
use crate::tunnel::{ssh_command, SshCfg};
use std::io::{self, Read, Write};
use std::path::PathBuf;
use std::process::Stdio;
use std::time::Duration;

/// Where the Itda for the remote Mac comes from.
pub enum Source {
    /// The GitHub release of this version.
    Release(String),
    /// A local Itda.app zip (development and tests).
    Bundle(PathBuf),
}

#[derive(Debug, Default, PartialEq)]
pub struct Ready {
    pub port: u16,
    pub password: String,
    /// Screen Recording / Accessibility granted on the Mac.
    pub screen: bool,
    pub access: bool,
    /// Version installed by this run (None: the Mac already had a good one).
    pub installed: Option<String>,
}

/// SSH to this Mac: its SSH settings, else the file connection's account, else the Mac
/// account it signs in with (the same password works for Remote Login).
pub fn ssh_cfg(host: &crate::store::Host, login: &crate::store::Login) -> SshCfg {
    let (h, _) = crate::tunnel::split_host(&host.host, 5900);
    let user = [&host.ssh.user, &host.files.user, &login.user].into_iter().find(|u| !u.is_empty()).cloned().unwrap_or_default();
    SshCfg {
        host: if host.files.host.is_empty() { h } else { host.files.host.clone() },
        port: [host.ssh.port, host.files.port].into_iter().find(|p| *p != 0).unwrap_or(22),
        user,
        pass: login.pass.clone().unwrap_or_default(),
        vnc_port: if host.hp.port == 0 { crate::host_mac_port() } else { host.hp.port },
        key: login.ssh_key.clone(),
        key_pass: login.key_pass.clone().unwrap_or_default(),
    }
}

/// Keychain id of a host's high-performance sharing password.
pub fn password_key(id: &str) -> String {
    format!("hp-{id}")
}

const RELEASES: &str = "https://github.com/jiunbae/itda/releases/download";
const BUNDLE_ID: &str = "dev.jiun.itda";

/// Run `script` with bash on the Mac, returning stdout; with `upload`, the script is
/// instead the remote command and `upload` is its stdin (e.g. `cat > file`).
fn run(cfg: &SshCfg, script: &str, upload: Option<&mut dyn Read>) -> io::Result<String> {
    let mut cmd = ssh_command(cfg)?;
    cmd.arg(crate::tunnel::ssh_target(cfg)?).arg(if upload.is_some() { script } else { "/bin/bash -s" });
    let mut child = cmd.stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped()).spawn()?;
    {
        let mut stdin = child.stdin.take().unwrap();
        match upload {
            Some(input) => {
                io::copy(input, &mut stdin)?;
            }
            None => stdin.write_all(script.as_bytes())?,
        }
    }
    let out = child.wait_with_output()?;
    let text = String::from_utf8_lossy(&out.stdout).into_owned();
    if !out.status.success() {
        let err = String::from_utf8_lossy(&out.stderr);
        let why = text.lines().chain(err.lines()).rev().find(|l| l.starts_with("error=")).map(|l| l.trim_start_matches("error=").to_string());
        return Err(io::Error::other(why.unwrap_or_else(|| format!("원격 명령이 실패했습니다: {}", err.trim()))));
    }
    Ok(text)
}

fn field<'a>(text: &'a str, key: &str) -> Option<&'a str> {
    text.lines().find_map(|l| l.strip_prefix(key)?.strip_prefix('=')).map(str::trim)
}

fn version_at_least(v: &str, min: &str) -> bool {
    let p = |s: &str| s.split('.').map(|x| x.parse::<u32>().unwrap_or(0)).collect::<Vec<_>>();
    p(v) >= p(min)
}

/// The first Itda version that has high-performance sharing.
pub const MIN_VERSION: &str = env!("CARGO_PKG_VERSION");

/// Shell-quote for bash.
fn q(s: &str) -> String {
    format!("'{}'", s.replace('\'', r"'\''"))
}

/// (Screen Recording, Accessibility) as the Mac's Itda last reported them.
pub fn status(cfg: &SshCfg) -> io::Result<(bool, bool)> {
    let out = run(cfg, r#"cat "$HOME/Library/Application Support/Itda/hp-status" 2>/dev/null; true"#, None)?;
    let kv = |k: &str| out.split_whitespace().any(|p| p == format!("{k}=1"));
    Ok((kv("screen"), kv("access")))
}

pub fn upgrade(cfg: &SshCfg, source: &Source, progress: &dyn Fn(&str)) -> io::Result<Ready> {
    progress("Mac 확인 중");
    let probe = run(cfg, &format!(r#"
set -e
[ "$(uname -s)" = Darwin ] || {{ echo "error=Mac이 아닙니다"; exit 3; }}
me=$(id -un); console=$(stat -f%Su /dev/console)
[ "$me" = "$console" ] || {{ echo "error=이 Mac 화면에 로그인한 계정($console)으로 연결해야 고성능 모드를 켤 수 있습니다"; exit 3; }}
app=""
for a in /Applications/Itda.app "$HOME/Applications/Itda.app"; do [ -d "$a" ] && {{ app=$a; break; }}; done
echo "app=$app"
[ -n "$app" ] && echo "ver=$(defaults read "$app/Contents/Info" CFBundleShortVersionString 2>/dev/null)"
echo "arch=$(uname -m)"
"#), None)?;
    let have = field(&probe, "ver").filter(|v| !v.is_empty());
    let mut ready = Ready::default();
    let need = match (have, source) {
        (_, Source::Bundle(_)) => true,
        (Some(v), _) => !version_at_least(v, MIN_VERSION),
        (None, _) => true,
    };
    if need {
        progress("Itda 설치 중");
        // Install into /Applications when that's writable here, else ~/Applications; quit a
        // running Itda first so the new one replaces it.
        let install = format!(r#"
set -e
T=$(mktemp -d /tmp/itda-hp.XXXX)
{fetch}
ditto -x -k "$T/Itda.zip" "$T/x"
[ -d "$T/x/Itda.app" ] || {{ echo "error=설치 파일이 올바르지 않습니다"; exit 4; }}
dest=/Applications
[ -w /Applications ] || {{ dest="$HOME/Applications"; mkdir -p "$dest"; }}
[ -n {app} ] && dest=$(dirname {app})
if pgrep -f "$dest/Itda.app/Contents/MacOS/" >/dev/null; then osascript -e 'tell application id "{BUNDLE_ID}" to quit' >/dev/null 2>&1 || true; sleep 2; pkill -f "$dest/Itda.app/Contents/MacOS/" || true; fi
rm -rf "$dest/Itda.app.old"; [ -d "$dest/Itda.app" ] && mv "$dest/Itda.app" "$dest/Itda.app.old"
ditto "$T/x/Itda.app" "$dest/Itda.app" && rm -rf "$dest/Itda.app.old" "$T"
echo "installed=$(defaults read "$dest/Itda.app/Contents/Info" CFBundleShortVersionString)"
"#,
            app = q(field(&probe, "app").unwrap_or("")),
            fetch = match source {
                Source::Release(v) => format!(
                    r#"curl -fsSL {zip} -o "$T/Itda.zip" || {{ echo "error=Itda를 내려받지 못했습니다"; exit 4; }}
want=$(curl -fsSL {sums} | awk '$2=="Itda-macos-universal.zip"{{print $1}}')
got=$(shasum -a 256 "$T/Itda.zip" | cut -c1-64)
[ -n "$want" ] && [ "$want" = "$got" ] || {{ echo "error=내려받은 Itda의 체크섬이 맞지 않습니다"; exit 4; }}"#,
                    zip = q(&format!("{RELEASES}/v{v}/Itda-macos-universal.zip")),
                    sums = q(&format!("{RELEASES}/v{v}/SHA256SUMS.txt"))
                ),
                // Uploaded just before (see below).
                Source::Bundle(_) => r#"mv /tmp/itda-hp-upload.zip "$T/Itda.zip""#.to_string(),
            }
        );
        if let Source::Bundle(p) = source {
            run(cfg, "umask 077; cat > /tmp/itda-hp-upload.zip", Some(&mut std::fs::File::open(p)?))?;
        }
        let out = run(cfg, &install, None)?;
        ready.installed = field(&out, "installed").map(str::to_string);
    }
    progress("고성능 공유를 켜는 중");
    let out = run(cfg, &format!(r#"
set -e
data="$HOME/Library/Application Support/Itda"
mkdir -p "$data"; echo 1 > "$data/hp-sharing"; rm -f "$data/hp-status"
app=""
for a in /Applications/Itda.app "$HOME/Applications/Itda.app"; do [ -d "$a" ] && {{ app=$a; break; }}; done
[ -n "$app" ] || {{ echo "error=Itda가 설치되어 있지 않습니다"; exit 5; }}
# A running Itda started before the switch doesn't serve yet: restart it, hidden.
if pgrep -f "$app/Contents/MacOS/" >/dev/null && ! nc -z 127.0.0.1 5910 2>/dev/null; then
  osascript -e 'tell application id "{BUNDLE_ID}" to quit' >/dev/null 2>&1 || true; sleep 2; pkill -f "$app/Contents/MacOS/" || true
fi
pgrep -f "$app/Contents/MacOS/" >/dev/null || open -g -j "$app"
for i in $(seq 1 60); do [ -f "$data/hp-status" ] && [ -f "$data/hp-password" ] && break; sleep 0.5; done
[ -f "$data/hp-status" ] || {{ echo "error=Itda가 고성능 공유를 시작하지 못했습니다"; exit 6; }}
cat "$data/hp-status"
echo "password=$(cat "$data/hp-password")"
"#), None)?;
    let status = out.lines().find(|l| l.starts_with("screen=")).unwrap_or("");
    let kv = |k: &str| status.split_whitespace().find_map(|p| p.strip_prefix(k)?.strip_prefix('=')).unwrap_or("");
    ready.screen = kv("screen") == "1";
    ready.access = kv("access") == "1";
    ready.port = kv("port").parse().unwrap_or(crate::host_mac_port());
    ready.password = field(&out, "password").unwrap_or("").to_string();
    if ready.password.len() < 6 {
        return Err(io::Error::other("고성능 공유 암호를 읽지 못했습니다"));
    }
    std::thread::sleep(Duration::from_millis(200));
    Ok(ready)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versions_and_fields() {
        assert!(version_at_least("0.10.0", "0.9.6"));
        assert!(!version_at_least("0.9.5", "0.9.6"));
        assert!(version_at_least("0.9.6", "0.9.6"));
        assert_eq!(field("a=1\napp=/Applications/Itda.app\nver=0.9.6\n", "ver"), Some("0.9.6"));
        assert_eq!(q("it's"), r"'it'\''s'");
    }

    /// Against a real Mac over SSH: ITDA_HP_TEST_HOST=user@host, ITDA_HP_TEST_BUNDLE=Itda.zip.
    #[test]
    #[ignore]
    fn upgrade_a_real_mac() {
        let spec = std::env::var("ITDA_HP_TEST_HOST").expect("ITDA_HP_TEST_HOST");
        let (user, host) = spec.split_once('@').unwrap();
        let cfg = SshCfg { host: host.into(), port: 22, user: user.into(), pass: String::new(), vnc_port: 5900, key: String::new(), key_pass: String::new() };
        let source = match std::env::var("ITDA_HP_TEST_BUNDLE") {
            Ok(p) => Source::Bundle(p.into()),
            Err(_) => Source::Release(env!("CARGO_PKG_VERSION").into()),
        };
        let r = upgrade(&cfg, &source, &|s| println!("progress: {s}")).unwrap();
        let password = r.password.clone();
        println!("{:?}", Ready { password: "<hidden>".into(), ..r });

        // Then the viewer's way in: the gateway's SSH tunnel + VNC password, asking for H.264.
        use std::io::{Read, Write};
        let host = crate::store::Host { id: format!("hp-test-{}", crate::store::new_id()), host: host.into(), os: "mac".into(), ssh: crate::store::Ssh { on: false, user: user.into(), port: 0 }, hp: crate::store::Hp { on: true, port: 0 }, ..Default::default() };
        crate::store::set_password(&password_key(&host.id), Some(&password));
        let opened = crate::gateway::open_hp(&host, &crate::store::Login::default());
        crate::store::set_password(&password_key(&host.id), None);
        let (_tunnel, raw) = opened.expect("open_hp");
        let (mut r, mut w) = (raw.reader, raw.writer);
        println!("server init {}x{} {:?}", raw.width, raw.height, raw.name);
        let mut m = vec![2, 0, 0, 2];
        for e in [50i32, 16] {
            m.extend_from_slice(&e.to_be_bytes());
        }
        m.extend_from_slice(&[3, 0, 0, 0, 0, 0]);
        m.extend_from_slice(&raw.width.to_be_bytes());
        m.extend_from_slice(&raw.height.to_be_bytes());
        w.write_all(&m).unwrap();
        let mut hdr = [0u8; 4 + 12];
        r.read_exact(&mut hdr).unwrap();
        let enc = i32::from_be_bytes([hdr[12], hdr[13], hdr[14], hdr[15]]);
        println!("first rectangle encoding {enc}");
        assert_eq!((hdr[0], enc), (0, 50), "the first update should be H.264");
    }
}
