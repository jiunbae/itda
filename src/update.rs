//! Self-update from GitHub Releases. Nothing installs on its own: a newer release is
//! announced to the page, and the person chooses to install it. The download is checked
//! against the release's SHA256SUMS.txt and, on macOS, against the code signature (a
//! Developer ID-signed Itda only accepts an update signed by the same team).
use serde_json::Value;
use sha2::{Digest, Sha256};
#[cfg(target_os = "macos")]
use std::path::PathBuf;
use std::{
    io,
    path::Path,
    process::{Command, Stdio},
};

pub const REPO: &str = "jiunbae/itda";
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
#[cfg(target_os = "macos")]
const ASSET: &str = "Itda-macos-universal.zip";
#[cfg(not(target_os = "macos"))]
const ASSET: &str = "itda.exe";
/// The Apple team that signs Itda releases.
#[cfg(target_os = "macos")]
const TEAM: &str = "728FW73BS8";

#[derive(Clone, Debug, PartialEq)]
pub struct Release {
    pub version: String,
    pub page: String,
    asset: String,
    sums: String,
}

fn parse(v: &str) -> Vec<u32> {
    v.trim_start_matches('v').split('.').map(|x| x.parse().unwrap_or(0)).collect()
}
pub fn newer(v: &str, than: &str) -> bool {
    parse(v) > parse(than)
}

fn curl() -> Command {
    let mut c = Command::new(if cfg!(windows) { "curl.exe" } else { "/usr/bin/curl" });
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        c.creation_flags(0x0800_0000);
    }
    c.args(["-sfL", "--proto", "=https", "--retry", "2"]).stdin(Stdio::null());
    c
}

/// The latest published release when it is newer than this Itda.
pub fn check() -> io::Result<Option<Release>> {
    let out = curl()
        .args(["--max-time", "20", "-H", "Accept: application/vnd.github+json"])
        .arg(format!("https://api.github.com/repos/{REPO}/releases/latest"))
        .output()?;
    if !out.status.success() {
        return Err(io::Error::other("업데이트 정보를 가져오지 못했습니다. 인터넷 연결을 확인해 주세요"));
    }
    release(&serde_json::from_slice(&out.stdout)?)
}

fn release(v: &Value) -> io::Result<Option<Release>> {
    if v["draft"] == true || v["prerelease"] == true {
        return Ok(None);
    }
    let version = v["tag_name"].as_str().unwrap_or("").trim_start_matches('v').to_string();
    if !newer(&version, VERSION) {
        return Ok(None);
    }
    let url = |name: &str| {
        v["assets"].as_array()?.iter().find(|a| a["name"] == name)?["browser_download_url"].as_str().map(str::to_string)
    };
    let (Some(asset), Some(sums)) = (url(ASSET), url("SHA256SUMS.txt")) else { return Ok(None) };
    let ours = format!("https://github.com/{REPO}/releases/download/");
    if !asset.starts_with(&ours) || !sums.starts_with(&ours) {
        return Err(io::Error::other("업데이트 주소가 올바르지 않습니다"));
    }
    let page = v["html_url"].as_str().filter(|u| u.starts_with(&format!("https://github.com/{REPO}/"))).unwrap_or("").to_string();
    Ok(Some(Release { version, page, asset, sums }))
}

fn download(url: &str, to: &Path) -> io::Result<()> {
    let ok = curl().args(["--max-time", "900", "-o"]).arg(to).arg(url).stdout(Stdio::null()).stderr(Stdio::null()).status()?.success();
    if ok { Ok(()) } else { Err(io::Error::other("업데이트를 내려받지 못했습니다")) }
}

fn expected(sums: &str, name: &str) -> Option<String> {
    sums.lines().find_map(|l| {
        let mut it = l.split_whitespace();
        let (hash, file) = (it.next()?, it.next()?);
        (file.trim_start_matches('*') == name && hash.len() == 64).then(|| hash.to_ascii_lowercase())
    })
}

fn sha256(path: &Path) -> io::Result<String> {
    let mut h = Sha256::new();
    io::copy(&mut std::fs::File::open(path)?, &mut h)?;
    Ok(h.finalize().iter().map(|b| format!("{b:02x}")).collect())
}

/// Download and verify, then hand the swap to a helper that waits for this process to exit.
/// On success the caller quits Itda; the helper replaces it and starts the new version.
pub fn install(r: &Release, progress: &dyn Fn(&str)) -> io::Result<()> {
    let dir = std::env::temp_dir().join(format!("itda-update-{}", r.version));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir)?;
    progress("내려받는 중…");
    let sums = dir.join("SHA256SUMS.txt");
    let file = dir.join(ASSET);
    download(&r.sums, &sums)?;
    download(&r.asset, &file)?;
    progress("확인하는 중…");
    let want = expected(&std::fs::read_to_string(&sums)?, ASSET).ok_or_else(|| io::Error::other("업데이트 체크섬을 찾지 못했습니다"))?;
    if sha256(&file)? != want {
        let _ = std::fs::remove_dir_all(&dir);
        return Err(io::Error::other("내려받은 파일이 릴리스와 다릅니다. 다시 시도해 주세요"));
    }
    progress("설치하는 중…");
    swap(&dir, &file)
}

#[cfg(target_os = "macos")]
fn current_app() -> io::Result<PathBuf> {
    let exe = std::env::current_exe()?;
    exe.ancestors()
        .nth(3)
        .filter(|a| a.extension().is_some_and(|e| e == "app"))
        .map(Path::to_path_buf)
        .ok_or_else(|| io::Error::other("응용 프로그램 폴더의 Itda 앱에서 업데이트해 주세요"))
}

/// TeamIdentifier of a signed bundle; None for ad-hoc.
#[cfg(target_os = "macos")]
fn team(app: &Path) -> Option<String> {
    let out = Command::new("/usr/bin/codesign").args(["-dv", "--verbose=2"]).arg(app).output().ok()?;
    String::from_utf8_lossy(&out.stderr)
        .lines()
        .find_map(|l| l.strip_prefix("TeamIdentifier="))
        .filter(|t| *t != "not set")
        .map(str::to_string)
}

/// Whether a new bundle may replace the running one: a signed Itda only takes the same team.
#[cfg(target_os = "macos")]
fn trusted(current: Option<&str>, new: Option<&str>) -> bool {
    match (current, new) {
        (_, Some(t)) if t != TEAM => false,
        (Some(_), None) => false,
        _ => true,
    }
}

#[cfg(target_os = "macos")]
fn swap(dir: &Path, zip: &Path) -> io::Result<()> {
    let app = current_app()?;
    let new = stage(&app, dir, zip)?;
    replace_after(std::process::id(), &app, &new, dir, true)
}

/// Unpack and check the new bundle beside the download; the running app is untouched.
#[cfg(target_os = "macos")]
fn stage(app: &Path, dir: &Path, zip: &Path) -> io::Result<PathBuf> {
    let parent = app.parent().ok_or_else(|| io::Error::other("앱 위치를 확인하지 못했습니다"))?;
    let probe = parent.join(".itda-update-probe");
    std::fs::write(&probe, b"").map_err(|_| io::Error::other(format!("{}에 쓸 수 없습니다. 관리자 계정으로 업데이트해 주세요", parent.display())))?;
    let _ = std::fs::remove_file(&probe);
    let x = dir.join("x");
    if !Command::new("/usr/bin/ditto").arg("-x").arg("-k").arg(zip).arg(&x).status()?.success() {
        return Err(io::Error::other("업데이트 파일을 풀지 못했습니다"));
    }
    let new = x.join("Itda.app");
    if !Command::new("/usr/bin/codesign").args(["--verify", "--deep", "--strict"]).arg(&new).stderr(Stdio::null()).status()?.success() {
        return Err(io::Error::other("업데이트의 서명을 확인하지 못했습니다"));
    }
    if !trusted(team(app).as_deref(), team(&new).as_deref()) {
        return Err(io::Error::other("업데이트의 서명이 Itda와 다릅니다. 설치하지 않았습니다"));
    }
    Ok(new)
}

/// Once `pid` has exited, swap the bundles (restoring the old one if that fails) and start it.
#[cfg(target_os = "macos")]
fn replace_after(pid: u32, app: &Path, new: &Path, dir: &Path, launch: bool) -> io::Result<()> {
    let script = r#"while kill -0 "$1" 2>/dev/null; do sleep 0.2; done
rm -rf "$2.old"
if mv "$2" "$2.old" && /usr/bin/ditto "$3" "$2"; then rm -rf "$2.old"; else rm -rf "$2"; mv "$2.old" "$2"; fi
[ "$5" = 1 ] && /usr/bin/open "$2"
rm -rf "$4""#;
    Command::new("/usr/bin/nohup")
        .args(["/bin/sh", "-c", script, "itda-update"])
        .arg(pid.to_string())
        .arg(app)
        .arg(new)
        .arg(dir)
        .arg(if launch { "1" } else { "0" })
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()?;
    Ok(())
}

/// Windows lets a running exe be renamed: move it aside, put the new one in place, and
/// start it once this process has exited (Itda allows one instance).
#[cfg(not(target_os = "macos"))]
fn swap(dir: &Path, new: &Path) -> io::Result<()> {
    let exe = std::env::current_exe()?;
    let old = exe.with_extension("exe.old");
    let _ = std::fs::remove_file(&old);
    std::fs::rename(&exe, &old).map_err(|_| io::Error::other(format!("{}에 쓸 수 없습니다", exe.display())))?;
    if let Err(e) = std::fs::copy(new, &exe) {
        let _ = std::fs::rename(&old, &exe);
        return Err(e);
    }
    let _ = std::fs::remove_dir_all(dir);
    let mut c = Command::new("powershell.exe");
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        c.creation_flags(0x0800_0000);
    }
    c.args(["-NoProfile", "-NonInteractive", "-Command", "param($p,$e) Wait-Process -Id $p -ErrorAction SilentlyContinue; Start-Process -FilePath $e"])
        .arg(std::process::id().to_string())
        .arg(&exe)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()?;
    Ok(())
}

/// Leftovers of a finished update.
pub fn cleanup() {
    #[cfg(not(target_os = "macos"))]
    if let Ok(exe) = std::env::current_exe() {
        let _ = std::fs::remove_file(exe.with_extension("exe.old"));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn versions() {
        assert!(newer("0.9.12", "0.9.11"));
        assert!(newer("v0.10.0", "0.9.11"));
        assert!(!newer("0.9.11", "0.9.11"));
        assert!(!newer("0.9.9", "0.9.11"));
    }

    #[test]
    fn release_fields() {
        let base = format!("https://github.com/{REPO}/releases/download/v99.0.0/");
        let v = json!({"tag_name":"v99.0.0","html_url":format!("https://github.com/{REPO}/releases/tag/v99.0.0"),"assets":[{"name":ASSET,"browser_download_url":format!("{base}{ASSET}")},{"name":"SHA256SUMS.txt","browser_download_url":format!("{base}SHA256SUMS.txt")}]});
        let r = release(&v).unwrap().unwrap();
        assert_eq!(r.version, "99.0.0");
        assert!(release(&json!({"tag_name":format!("v{VERSION}"),"assets":v["assets"]})).unwrap().is_none());
        assert!(release(&json!({"tag_name":"v99.0.0","prerelease":true,"assets":v["assets"]})).unwrap().is_none());
        let elsewhere = json!({"tag_name":"v99.0.0","assets":[{"name":ASSET,"browser_download_url":"https://example.com/x"},{"name":"SHA256SUMS.txt","browser_download_url":format!("{base}SHA256SUMS.txt")}]});
        assert!(release(&elsewhere).is_err());
    }

    #[test]
    fn checksums() {
        let h = "a".repeat(64);
        let sums = format!("{h}  Itda-macos-universal.zip\n{}  itda.exe\n", "B".repeat(64));
        assert_eq!(expected(&sums, "Itda-macos-universal.zip").as_deref(), Some(h.as_str()));
        assert_eq!(expected(&sums, "itda.exe").unwrap(), "b".repeat(64));
        assert!(expected(&sums, "other.zip").is_none());
    }

    /// Real release, scratch bundle: download, verify, stage and swap once a stand-in
    /// process exits. Never touches /Applications and never launches anything.
    ///   ITDA_UPDATE_TEST_TAG=v0.9.11 cargo test real_release_swap -- --ignored --nocapture
    #[cfg(target_os = "macos")]
    #[test]
    #[ignore]
    fn real_release_swap() {
        let tag = std::env::var("ITDA_UPDATE_TEST_TAG").expect("ITDA_UPDATE_TEST_TAG");
        let base = format!("https://github.com/{REPO}/releases/download/{tag}/");
        let r = Release { version: tag.trim_start_matches('v').into(), page: String::new(), asset: format!("{base}{ASSET}"), sums: format!("{base}SHA256SUMS.txt") };
        let root = std::env::temp_dir().join(format!("itda-update-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let app = root.join("Apps/Itda.app");
        std::fs::create_dir_all(app.join("Contents")).unwrap();
        std::fs::write(app.join("Contents/marker"), b"old").unwrap();
        let dir = root.join("download");
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join(ASSET);
        download(&r.sums, &dir.join("SHA256SUMS.txt")).unwrap();
        download(&r.asset, &file).unwrap();
        assert_eq!(sha256(&file).unwrap(), expected(&std::fs::read_to_string(dir.join("SHA256SUMS.txt")).unwrap(), ASSET).unwrap());
        let new = stage(&app, &dir, &file).unwrap();
        let mut stand_in = Command::new("/bin/sleep").arg("1").spawn().unwrap();
        replace_after(stand_in.id(), &app, &new, &dir, false).unwrap();
        assert!(app.join("Contents/marker").exists(), "swapped before the app exited");
        stand_in.wait().unwrap();
        for _ in 0..100 { if !dir.exists() { break; } std::thread::sleep(std::time::Duration::from_millis(100)); }
        let plist = std::fs::read_to_string(app.join("Contents/Info.plist")).unwrap();
        assert!(plist.contains(&format!("<string>{}</string>", r.version)), "new bundle not in place");
        assert!(!app.join("Contents/marker").exists() && !root.join("Apps/Itda.app.old").exists());
        assert!(Command::new("/usr/bin/codesign").args(["--verify", "--deep", "--strict"]).arg(&app).status().unwrap().success());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn signature_policy() {
        assert!(trusted(None, None));
        assert!(trusted(None, Some(TEAM)));
        assert!(trusted(Some(TEAM), Some(TEAM)));
        assert!(!trusted(Some(TEAM), None));
        assert!(!trusted(None, Some("OTHERTEAM1")));
        assert!(!trusted(Some(TEAM), Some("OTHERTEAM1")));
    }
}
