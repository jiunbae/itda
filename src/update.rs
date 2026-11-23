//! Self-update from GitHub Releases. Nothing installs on its own: a newer release is
//! announced to the page, and the person chooses to install it. The download is checked
//! against the release's SHA256SUMS.txt and, on macOS, against the code signature (a
//! Developer ID-signed Itda only accepts an update signed by the same team).
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{
    fs, io,
    path::{Path, PathBuf},
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
    let mut c = Command::new(if cfg!(windows) { system_tool("curl.exe") } else { "/usr/bin/curl".into() });
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
        return Err(io::Error::other(tr!("update.checkFailed")));
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
        return Err(io::Error::other(tr!("update.badUrl")));
    }
    let page = v["html_url"].as_str().filter(|u| u.starts_with(&format!("https://github.com/{REPO}/"))).unwrap_or("").to_string();
    Ok(Some(Release { version, page, asset, sums }))
}

fn download(url: &str, to: &Path) -> io::Result<()> {
    let ok = curl().args(["--max-time", "900", "-o"]).arg(to).arg(url).stdout(Stdio::null()).stderr(Stdio::null()).status()?.success();
    if ok { Ok(()) } else { Err(io::Error::other(tr!("update.downloadFailed"))) }
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

/// A scratch directory owned by this attempt until the helper takes over.
struct UpdateDir {
    path: PathBuf,
    handed_off: bool,
}

impl UpdateDir {
    fn create(parent: &Path) -> io::Result<Self> {
        let mut random = [0; 16];
        getrandom::getrandom(&mut random).map_err(io::Error::other)?;
        let path = parent.join(format!(".itda-update-{:032x}", u128::from_le_bytes(random)));
        #[cfg(unix)]
        {
            use std::os::unix::fs::DirBuilderExt;
            fs::DirBuilder::new().mode(0o700).create(&path)?;
        }
        #[cfg(not(unix))]
        fs::create_dir(&path)?;
        Ok(Self { path, handed_off: false })
    }
}

impl Drop for UpdateDir {
    fn drop(&mut self) {
        if !self.handed_off {
            let _ = fs::remove_dir_all(&self.path);
        }
    }
}

/// Download and verify, then hand the swap to a helper that waits for this process to exit.
/// On success the caller quits Itda; the helper replaces it and starts the new version.
pub fn install(r: &Release, progress: &dyn Fn(&str)) -> io::Result<()> {
    #[cfg(windows)]
    let exe = std::env::current_exe()?;
    // Keep the Windows candidate and backup on the install volume so the helper
    // can use renames after shutdown without exposing a partially copied exe.
    #[cfg(windows)]
    let mut staging = UpdateDir::create(exe.parent().ok_or_else(|| io::Error::other(tr!("update.noAppPath")))?)
        .map_err(|e| io::Error::new(e.kind(), format!("{}: {e}", tr!("update.notWritable", path = exe.display()))))?;
    #[cfg(not(windows))]
    let mut staging = UpdateDir::create(&std::env::temp_dir())?;
    let dir = &staging.path;
    progress(tr!("update.downloading"));
    let sums = dir.join("SHA256SUMS.txt");
    let file = dir.join(ASSET);
    download(&r.sums, &sums)?;
    download(&r.asset, &file)?;
    progress(tr!("update.verifying"));
    let want = expected(&fs::read_to_string(&sums)?, ASSET).ok_or_else(|| io::Error::other(tr!("update.noChecksum")))?;
    if sha256(&file)? != want {
        return Err(io::Error::other(tr!("update.checksumMismatch")));
    }
    progress(tr!("update.installing"));
    #[cfg(windows)]
    drop(windows_helper(std::process::id(), &exe, dir, &failure_note())?);
    #[cfg(target_os = "macos")]
    swap(dir, &file)?;
    #[cfg(not(any(windows, target_os = "macos")))]
    return Err(io::Error::from(io::ErrorKind::Unsupported));
    #[cfg(any(windows, target_os = "macos"))]
    {
        staging.handed_off = true;
        Ok(())
    }
}

#[cfg(target_os = "macos")]
fn current_app() -> io::Result<PathBuf> {
    let exe = std::env::current_exe()?;
    exe.ancestors()
        .nth(3)
        .filter(|a| a.extension().is_some_and(|e| e == "app"))
        .map(Path::to_path_buf)
        .ok_or_else(|| io::Error::other(tr!("update.notInApplications")))
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
    let parent = app.parent().ok_or_else(|| io::Error::other(tr!("update.noAppPath")))?;
    let probe = parent.join(".itda-update-probe");
    std::fs::write(&probe, b"").map_err(|_| io::Error::other(tr!("update.notWritableAdmin", path = parent.display())))?;
    let _ = std::fs::remove_file(&probe);
    let x = dir.join("x");
    if !Command::new("/usr/bin/ditto").arg("-x").arg("-k").arg(zip).arg(&x).status()?.success() {
        return Err(io::Error::other(tr!("update.unzipFailed")));
    }
    let new = x.join("Itda.app");
    if !Command::new("/usr/bin/codesign").args(["--verify", "--deep", "--strict"]).arg(&new).stderr(Stdio::null()).status()?.success() {
        return Err(io::Error::other(tr!("update.signatureFailed")));
    }
    if !trusted(team(app).as_deref(), team(&new).as_deref()) {
        return Err(io::Error::other(tr!("update.signatureMismatch")));
    }
    Ok(new)
}

/// $1 pid to wait for, $2 app, $3 new bundle, $4 download dir, $5 launch (1/0), $6 failure note,
/// $7-$10 the notes for: a stuck backup, a stuck app, an unrestorable app, a failed copy.
/// The current app is only removed after it was moved aside; any failure leaves (or puts back)
/// the current app, keeps the download for inspection, writes the note and still relaunches.
#[cfg(target_os = "macos")]
const HELPER: &str = r#"pid=$1 app=$2 new=$3 dl=$4 launch=$5 note=$6
while kill -0 "$pid" 2>/dev/null; do sleep 0.2; done
fail() { printf '%s\n' "$1" > "$note"; [ "$launch" = 1 ] && /usr/bin/open "$app"; exit 1; }
if [ -e "$app.old" ]; then rm -rf "$app.old" || fail "$7"; fi
[ -e "$app.old" ] && fail "$7"
mv "$app" "$app.old" || fail "$8"
if ! /usr/bin/ditto "$new" "$app"; then
  rm -rf "$app"
  mv "$app.old" "$app" || { printf '%s\n' "$9" > "$note"; exit 1; }
  fail "${10}"
fi
rm -rf "$app.old" "$dl"
[ "$launch" = 1 ] && /usr/bin/open "$app"
exit 0"#;

/// Once `pid` has exited, swap the bundles (restoring the old one if that fails) and start it.
#[cfg(target_os = "macos")]
fn replace_after(pid: u32, app: &Path, new: &Path, dir: &Path, launch: bool) -> io::Result<()> {
    let note = failure_note();
    let _ = std::fs::remove_file(&note);
    let script = HELPER;
    Command::new("/usr/bin/nohup")
        .args(["/bin/sh", "-c", script, "itda-update"])
        .arg(pid.to_string())
        .arg(app)
        .arg(new)
        .arg(dir)
        .arg(if launch { "1" } else { "0" })
        .arg(&note)
        .arg(tr!("update.backupStuck", backup = format!("{}.old", app.display())))
        .arg(tr!("update.moveFailed", app = app.display()))
        .arg(tr!("update.restoreFailed", backup = format!("{}.old", app.display()), app = app.display()))
        .arg(tr!("update.copyFailed"))
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()?;
    Ok(())
}

/// The helper opens the original process before signaling readiness. Both executable
/// moves happen after that process exits and Rust has authorized the handoff.
#[cfg(windows)]
const WINDOWS_HELPER: &str = r#"$ErrorActionPreference = 'Stop'
$appPath = $env:ITDA_UPDATE_EXE
$stagingDir = $env:ITDA_UPDATE_DIR
$candidate = [IO.Path]::Combine($stagingDir, 'itda.exe')
$backup = [IO.Path]::Combine($stagingDir, 'previous.exe')

function Clear-Staging {
    try { [IO.Directory]::Delete($stagingDir, $true) } catch {}
}

function Save-Failure([string]$message) {
    try {
        [IO.File]::WriteAllText($env:ITDA_UPDATE_NOTE, $message)
        return $true
    } catch {
        try { [IO.File]::WriteAllText([IO.Path]::Combine($stagingDir, 'update-failed'), $message) } catch {}
        return $false
    }
}

function Start-App {
    $start = [Diagnostics.ProcessStartInfo]::new()
    $start.FileName = $appPath
    $start.WorkingDirectory = [IO.Path]::GetDirectoryName($appPath)
    $start.UseShellExecute = $false
    $start.CreateNoWindow = $true
    [void][Diagnostics.Process]::Start($start)
}

$installLock = [Threading.Mutex]::new($false, $env:ITDA_UPDATE_LOCK)
if (!$installLock.WaitOne(0)) { exit 1 }
$original = [Diagnostics.Process]::GetProcessById([int]$env:ITDA_UPDATE_PID)
[void]$original.Handle
[IO.File]::WriteAllText([IO.Path]::Combine($stagingDir, 'ready'), '')
try {
    $original.WaitForExit()
} catch {
    [void](Save-Failure ($_.Exception.GetBaseException().Message))
    exit 1
}
if (![IO.File]::Exists([IO.Path]::Combine($stagingDir, 'commit'))) {
    Clear-Staging
    exit 1
}

$movedOld = $false
$movedNew = $false
try {
    [IO.File]::Move($appPath, $backup)
    $movedOld = $true
    [IO.File]::Move($candidate, $appPath)
    $movedNew = $true
    Start-App
} catch {
    $message = $_.Exception.GetBaseException().Message
    try {
        if ($movedNew) { [IO.File]::Move($appPath, $candidate) }
        if ($movedOld) { [IO.File]::Move($backup, $appPath) }
    } catch {
        [void](Save-Failure ($env:ITDA_UPDATE_RESTORE_FAILED + [Environment]::NewLine + $message + [Environment]::NewLine + $_.Exception.GetBaseException().Message))
        exit 1
    }
    $saved = Save-Failure $message
    try {
        Start-App
    } catch {
        [void](Save-Failure ($message + [Environment]::NewLine + $_.Exception.GetBaseException().Message))
        exit 1
    }
    if ($saved) { Clear-Staging }
    exit 1
}
Clear-Staging
exit 0
"#;

#[cfg(windows)]
fn windows_helper(pid: u32, exe: &Path, dir: &Path, note: &Path) -> io::Result<std::process::Child> {
    use std::os::windows::process::CommandExt;
    use std::time::{Duration, Instant};

    let backup = dir.join("previous.exe");
    let install_path = fs::canonicalize(exe)?;
    let lock_id = Sha256::digest(install_path.to_string_lossy().to_uppercase().as_bytes());
    let mut c = Command::new(system_tool("powershell.exe"));
    // The command is fixed. Pass all paths as data so PowerShell uses them literally.
    let mut child = c.creation_flags(0x0800_0000)
        .args(["-NoProfile", "-NonInteractive", "-Command", WINDOWS_HELPER])
        .env("ITDA_UPDATE_PID", pid.to_string())
        .env("ITDA_UPDATE_EXE", exe)
        .env("ITDA_UPDATE_DIR", dir)
        .env("ITDA_UPDATE_LOCK", format!("Global\\itda-update-{lock_id:x}"))
        .env("ITDA_UPDATE_NOTE", std::path::absolute(note)?)
        .env("ITDA_UPDATE_RESTORE_FAILED", tr!("update.restoreFailed", backup = backup.display(), app = exe.display()))
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()?;
    let ready = dir.join("ready");
    let end = Instant::now() + Duration::from_secs(15);
    let handoff = (|| {
        loop {
            if let Some(status) = child.try_wait()? {
                return Err(io::Error::other(format!("PowerShell update helper: {status}")));
            }
            if ready.try_exists()? {
                fs::File::create(dir.join("commit"))?;
                return Ok(());
            }
            if Instant::now() >= end {
                return Err(io::Error::new(io::ErrorKind::TimedOut, "PowerShell update helper readiness timed out"));
            }
            std::thread::sleep(Duration::from_millis(25));
        }
    })();
    if let Err(e) = handoff {
        let _ = child.kill();
        let _ = child.wait();
        return Err(e);
    }
    Ok(child)
}

/// Where the helper explains a failed swap; read (and removed) at the next launch.
pub fn failure_note() -> std::path::PathBuf {
    crate::store::dir().join("update-failed")
}
pub fn take_failure() -> Option<String> {
    let note = failure_note();
    let text = std::fs::read_to_string(&note).ok()?;
    let _ = std::fs::remove_file(&note);
    Some(text.trim().to_string()).filter(|t| !t.is_empty())
}

/// Remove the fixed backup left by earlier Windows updaters.
pub fn cleanup() {
    #[cfg(windows)]
    if let Ok(exe) = std::env::current_exe() {
        let _ = std::fs::remove_file(exe.with_extension("exe.old"));
    }
}

/// A Windows tool from the system folder by full path, so a file of the same name next to
/// Itda (the default search looks there first) can't stand in for it. Elsewhere: the name.
pub fn system_tool(exe: &str) -> std::path::PathBuf {
    #[cfg(windows)]
    {
        let root = std::env::var_os("SystemRoot").map(std::path::PathBuf::from).unwrap_or_else(|| r"C:\Windows".into());
        let sub = if exe.eq_ignore_ascii_case("powershell.exe") { r"System32\WindowsPowerShell\v1.0" } else { "System32" };
        return root.join(sub).join(exe);
    }
    #[allow(unreachable_code)]
    std::path::PathBuf::from(exe)
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

    #[test]
    fn staging_isolates_attempts_and_cleans_up_before_handoff() {
        let root = UpdateDir::create(&std::env::temp_dir()).unwrap();
        let first = UpdateDir::create(&root.path).unwrap();
        let mut second = UpdateDir::create(&root.path).unwrap();
        let first_path = first.path.clone();
        let second_path = second.path.clone();
        assert_ne!(first_path, second_path);
        fs::write(first_path.join(ASSET), b"partial download").unwrap();
        fs::write(second_path.join(ASSET), b"verified download").unwrap();

        drop(first);
        second.handed_off = true;
        drop(second);
        assert!(!first_path.exists());
        assert_eq!(fs::read(second_path.join(ASSET)).unwrap(), b"verified download");
    }

    #[cfg(windows)]
    #[test]
    fn windows_helper_rejection_leaves_the_installation_unchanged() {
        let root = UpdateDir::create(&std::env::temp_dir()).unwrap();
        let exe = root.path.join("Itda.exe");
        let note = root.path.join("failure");
        fs::write(&exe, b"current").unwrap();
        let stage = UpdateDir::create(&root.path).unwrap();
        fs::write(stage.path.join(ASSET), b"replacement").unwrap();

        assert!(windows_helper(u32::MAX, &exe, &stage.path, &note).is_err());
        assert_eq!(fs::read(exe).unwrap(), b"current");
        assert!(!stage.path.join("commit").exists());
        assert!(!note.exists());
    }

    #[cfg(windows)]
    #[test]
    fn windows_helper_waits_then_swaps_or_restores_the_executable() {
        use std::os::windows::process::CommandExt;

        let root = UpdateDir::create(&std::env::temp_dir()).unwrap();
        let app_dir = root.path.join("Test [Itda] 앱");
        fs::create_dir(&app_dir).unwrap();
        let current = fs::read(system_tool("whoami.exe")).unwrap();
        let replacement = fs::read(system_tool("hostname.exe")).unwrap();
        for (case, candidate) in [None, Some(&b"invalid executable"[..]), Some(&replacement[..])].into_iter().enumerate() {
            let exe = app_dir.join(format!("Itda portable {case}.exe"));
            let note = app_dir.join(format!("failure-{case}"));
            fs::write(&exe, &current).unwrap();
            let stage = UpdateDir::create(&app_dir).unwrap();
            if let Some(candidate) = candidate {
                fs::write(stage.path.join(ASSET), candidate).unwrap();
            }
            let mut parent = Command::new(system_tool("cmd.exe"))
                .creation_flags(0x0800_0000)
                .args(["/D", "/C", "pause"])
                .stdin(Stdio::piped())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
                .unwrap();
            let helper = windows_helper(parent.id(), &exe, &stage.path, &note);
            let competing = if candidate == Some(&replacement[..]) && helper.is_ok() {
                let other_stage = UpdateDir::create(&app_dir).unwrap();
                windows_helper(parent.id(), &exe, &other_stage.path, &note).err()
            } else {
                None
            };
            let unchanged = fs::read(&exe).unwrap() == current;
            let backup_exists = stage.path.join("previous.exe").exists();
            parent.kill().unwrap();
            parent.wait().unwrap();
            let status = helper.unwrap().wait().unwrap();

            assert!(unchanged && !backup_exists, "swapped before the process exited");
            assert!(!stage.path.exists(), "left staging after successful restart");
            if candidate == Some(&replacement[..]) {
                assert!(competing.is_some(), "another helper accepted the same executable");
                assert!(status.success());
                assert_eq!(fs::read(&exe).unwrap(), replacement);
                assert!(!note.exists());
            } else {
                assert!(!status.success());
                assert_eq!(fs::read(&exe).unwrap(), current);
                assert!(!fs::read_to_string(&note).unwrap().trim().is_empty());
                fs::remove_file(&note).unwrap();
            }
        }
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

    /// The swap helper itself, on scratch bundles: success, a stuck old backup, a failed copy.
    #[cfg(target_os = "macos")]
    #[test]
    fn helper_never_loses_the_current_app() {
        let root = std::env::temp_dir().join(format!("itda-helper-test-{}", std::process::id()));
        let run = |case: &str, setup: &dyn Fn(&Path)| -> (bool, String, Option<String>, bool) {
            let d = root.join(case);
            let (app, new, dl, note) = (d.join("Itda.app"), d.join("stage/Itda.app"), d.join("dl"), d.join("note"));
            std::fs::create_dir_all(&app).unwrap();
            std::fs::write(app.join("who"), "current").unwrap();
            std::fs::create_dir_all(&new).unwrap();
            std::fs::write(new.join("who"), "new").unwrap();
            std::fs::create_dir_all(&dl).unwrap();
            setup(&d);
            let ok = Command::new("/bin/sh").args(["-c", HELPER, "t", "2147483646"]).arg(&app).arg(&new).arg(&dl).arg("0").arg(&note).status().unwrap().success();
            let who = std::fs::read_to_string(app.join("who")).unwrap_or_default();
            (ok, who, std::fs::read_to_string(&note).ok(), dl.exists())
        };
        let (ok, who, note, dl) = run("normal", &|_| {});
        assert!(ok && who == "new" && note.is_none() && !dl, "normal swap");
        let (ok, who, note, dl) = run("stale", &|d| {
            let old = d.join("Itda.app.old");
            std::fs::create_dir_all(&old).unwrap();
            std::fs::write(old.join("who"), "stale").unwrap();
            Command::new("/bin/chmod").arg("0500").arg(&old).status().unwrap();
        });
        assert!(!ok && who == "current" && note.is_some() && dl, "stuck backup must keep the current app: ok={ok} who={who} note={note:?} dl={dl}");
        let (ok, who, note, _) = run("copy", &|d| { std::fs::remove_dir_all(d.join("stage")).unwrap(); });
        assert!(!ok && who == "current" && note.is_some(), "failed copy must restore the current app: {who}");
        Command::new("/bin/chmod").arg("-R").arg("u+w").arg(&root).status().unwrap();
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
