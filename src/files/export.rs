//! Files handed to Finder are real file URLs or lazy native file promises, never text.
use super::*;

#[derive(Clone)]
pub struct File {
    pub id: String,
    pub remote: bool,
    pub source: String,
    pub name: String,
    #[cfg_attr(windows, allow(dead_code))]
    pub dir: bool,
}
pub fn selection(v: &Value) -> io::Result<Vec<File>> {
    let side = get(v, "side");
    if side != "local" && side != "remote" {
        return Err(error(tr!("fx.export.place")));
    }
    let items = v["items"]
        .as_array()
        .ok_or_else(|| error(tr!("fx.chooseFile")))?;
    if items.is_empty() || items.len() > 10_000 {
        return Err(error(tr!("fx.export.count")));
    }
    let mut seen = std::collections::HashSet::new();
    items
        .iter()
        .map(|item| {
            let n = get(item, "name");
            name(&n)?;
            if !seen.insert(n.clone()) {
                return Err(error(tr!("fx.export.duplicate")));
            }
            Ok(File {
                id: super::scoped_id(&get(v, "id"), &v["window"]),
                remote: side == "remote",
                source: get(v, "path"),
                name: n,
                dir: item["dir"] == true,
            })
        })
        .collect()
}
impl File {
    pub fn path(&self) -> io::Result<PathBuf> {
        name(&self.name)?;
        Ok(if self.remote {
            PathBuf::from(join(&self.source, &self.name)?)
        } else {
            Path::new(&canonical(&self.source)?).join(&self.name)
        })
    }
}
#[cfg(target_os = "macos")]
/// Copy requested remote files to a private, persistent staging directory for the
/// system clipboard. Retain it after quitting so Finder can paste after Itda closes;
/// [`sweep`] bounds how many finished exports outlive their clipboard use.
///
/// Staging happens in `<id>.partial`, guarded by an exclusive lock on `<id>.lock`, and
/// is renamed to `<id>` only when complete. A crash drops the lock, so a sweep can tell
/// an abandoned partial from one another export (or another Itda) is still writing.
pub fn prepare(files: &[File], emit: Emit) -> io::Result<Vec<PathBuf>> {
    if files.iter().all(|f| !f.remote) {
        return files
            .iter()
            .map(|f| {
                let path = f.path()?;
                std::fs::symlink_metadata(&path)?;
                Ok(path)
            })
            .collect();
    }
    stage(&exports_dir(), files, |files, paths| {
        super::transfer::export_batch(files, paths, emit)
    })
}
#[cfg(target_os = "macos")]
pub fn exports_dir() -> PathBuf {
    crate::store::dir().join("file-exports")
}
#[cfg(target_os = "macos")]
const PARTIAL: &str = ".partial";
#[cfg(target_os = "macos")]
const LOCK: &str = ".lock";
/// Finished exports always kept, newest first, even when nothing references them.
#[cfg(target_os = "macos")]
const KEEP_NEWEST: usize = 3;
/// A finished export this young may still be on its way to the pasteboard.
#[cfg(target_os = "macos")]
const GRACE: std::time::Duration = std::time::Duration::from_secs(10 * 60);

#[cfg(target_os = "macos")]
fn private_dir(path: &Path) -> io::Result<()> {
    use std::os::unix::fs::DirBuilderExt;
    std::fs::DirBuilder::new().recursive(true).mode(0o700).create(path)
}
#[cfg(target_os = "macos")]
fn same_file(a: &std::fs::File, path: &Path) -> bool {
    use std::os::unix::fs::MetadataExt;
    match (a.metadata(), std::fs::metadata(path)) {
        (Ok(a), Ok(b)) => a.dev() == b.dev() && a.ino() == b.ino(),
        _ => false,
    }
}
/// Take `<id>.lock`, retrying if a sweep unlinked the file between open and lock.
#[cfg(target_os = "macos")]
fn hold_lock(path: &Path) -> io::Result<std::fs::File> {
    use std::os::unix::fs::OpenOptionsExt;
    for _ in 0..8 {
        let file = std::fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .mode(0o600)
            .open(path)?;
        file.lock()?;
        if same_file(&file, path) {
            return Ok(file);
        }
    }
    Err(io::Error::other("export staging lock kept changing"))
}
#[cfg(target_os = "macos")]
fn stage(
    exports: &Path,
    files: &[File],
    export: impl FnOnce(&[File], &[PathBuf]) -> io::Result<()>,
) -> io::Result<Vec<PathBuf>> {
    private_dir(exports)?;
    let id = crate::store::new_id();
    let lock_path = exports.join(format!("{id}{LOCK}"));
    let lock = hold_lock(&lock_path)?;
    let partial = exports.join(format!("{id}{PARTIAL}"));
    let root = exports.join(&id);
    let staged = (|| {
        private_dir(&partial)?;
        let paths: Vec<_> = files.iter().map(|f| partial.join(&f.name)).collect();
        export(files, &paths)?;
        std::fs::rename(&partial, &root)?;
        // Rename keeps the old mtime; retention ages a finished export from now.
        let _ = std::fs::File::open(&root)
            .and_then(|d| d.set_modified(std::time::SystemTime::now()));
        Ok(files.iter().map(|f| root.join(&f.name)).collect())
    })();
    if staged.is_err() {
        let _ = std::fs::remove_dir_all(&partial);
    }
    let _ = std::fs::remove_file(&lock_path);
    drop(lock);
    staged
}

/// Bound the staged clipboard exports. Keeps every export containing a `referenced`
/// path (what the pasteboard holds now), the newest [`KEEP_NEWEST`], anything younger
/// than [`GRACE`], and partials whose writer still holds the lock. Removes the rest,
/// including partials and locks a crashed export left behind. Never follows symlinks
/// out of `exports` and never fails: an entry it cannot judge is kept.
#[cfg(target_os = "macos")]
pub fn sweep(exports: &Path, referenced: &[PathBuf]) {
    sweep_at(exports, referenced, std::time::SystemTime::now())
}
#[cfg(target_os = "macos")]
fn sweep_at(exports: &Path, referenced: &[PathBuf], now: std::time::SystemTime) {
    let Ok(entries) = std::fs::read_dir(exports) else { return };
    let protected: std::collections::HashSet<std::ffi::OsString> = referenced
        .iter()
        .filter_map(|p| p.strip_prefix(exports).ok())
        .filter_map(|rel| match rel.components().next() {
            Some(std::path::Component::Normal(c)) => Some(c.to_os_string()),
            _ => None,
        })
        .collect();
    let mut finished = Vec::new();
    for entry in entries.flatten() {
        let name = entry.file_name();
        let Some(text) = name.to_str() else { continue };
        let Ok(kind) = entry.file_type() else { continue };
        let path = entry.path();
        if let Some(id) = text.strip_suffix(PARTIAL) {
            if kind.is_dir() {
                reclaim_partial(exports, id, &path);
            }
        } else if let Some(id) = text.strip_suffix(LOCK) {
            // A lock whose partial is gone: either finished (its writer removes it) or
            // a crash between lock and partial. Only reclaim if nobody holds it.
            if kind.is_file() && !exports.join(format!("{id}{PARTIAL}")).exists() {
                if let Some(guard) = try_hold(&path) {
                    let _ = std::fs::remove_file(&path);
                    drop(guard);
                }
            }
        } else if kind.is_dir() && !protected.contains(&name) {
            let modified = entry
                .metadata()
                .and_then(|m| m.modified())
                .unwrap_or(now);
            finished.push((modified, path));
        }
    }
    finished.sort_by_key(|f| std::cmp::Reverse(f.0));
    for (modified, path) in finished.into_iter().skip(KEEP_NEWEST) {
        if now.duration_since(modified).is_ok_and(|age| age >= GRACE) {
            let _ = std::fs::remove_dir_all(path);
        }
    }
}
#[cfg(target_os = "macos")]
fn try_hold(path: &Path) -> Option<std::fs::File> {
    let file = std::fs::OpenOptions::new().write(true).open(path).ok()?;
    file.try_lock().ok()?;
    same_file(&file, path).then_some(file)
}
#[cfg(target_os = "macos")]
fn reclaim_partial(exports: &Path, id: &str, partial: &Path) {
    let lock_path = exports.join(format!("{id}{LOCK}"));
    // Missing lock: only a crash leaves a partial without one (the writer locks first).
    let guard = if lock_path.exists() {
        match try_hold(&lock_path) {
            Some(guard) => Some(guard),
            None => return,
        }
    } else {
        None
    };
    let _ = std::fs::remove_dir_all(partial);
    let _ = std::fs::remove_file(&lock_path);
    drop(guard);
}

#[cfg(target_os = "macos")]
pub fn write(file: &File, target: &Path, emit: Emit) -> io::Result<()> {
    super::transfer::export_batch(std::slice::from_ref(file), &[target.to_path_buf()], emit)
}

#[cfg(all(test, target_os = "macos"))]
mod retention_tests {
    use super::*;
    use std::time::{Duration, SystemTime};

    fn temp_exports(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir()
            .join(format!("itda-exports-{tag}-{}", crate::store::new_id()))
            .join("file-exports");
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }
    fn remote(name: &str) -> File {
        File { id: "t".into(), remote: true, source: "/remote".into(), name: name.into(), dir: false }
    }
    fn write_fixture(paths: &[PathBuf]) -> io::Result<()> {
        for p in paths {
            std::fs::write(p, b"staged")?;
        }
        Ok(())
    }
    fn finished(exports: &Path, id: &str, age: Duration) -> PathBuf {
        let dir = exports.join(id);
        std::fs::create_dir(&dir).unwrap();
        std::fs::write(dir.join("a.txt"), b"x").unwrap();
        std::fs::File::open(&dir).unwrap().set_modified(SystemTime::now() - age).unwrap();
        dir
    }
    fn names(exports: &Path) -> Vec<String> {
        let mut v: Vec<_> = std::fs::read_dir(exports)
            .unwrap()
            .map(|e| e.unwrap().file_name().into_string().unwrap())
            .collect();
        v.sort();
        v
    }

    #[test]
    fn staging_publishes_only_complete_exports_and_leaves_no_lock() {
        let exports = temp_exports("stage");
        let paths = stage(&exports, &[remote("한글.txt")], |_, p| write_fixture(p)).unwrap();
        assert_eq!(std::fs::read(&paths[0]).unwrap(), b"staged");
        let root = paths[0].parent().unwrap();
        assert_eq!(root.parent().unwrap(), exports);
        assert_eq!(names(&exports), [root.file_name().unwrap().to_str().unwrap()]);
        let age = SystemTime::now().duration_since(std::fs::metadata(root).unwrap().modified().unwrap());
        assert!(age.map_or(true, |a| a < GRACE), "a fresh export must start its grace period");

        let failed = stage(&exports, &[remote("b.txt")], |_, p| {
            write_fixture(p)?;
            Err(io::Error::other("download failed"))
        });
        assert!(failed.is_err());
        assert_eq!(names(&exports).len(), 1, "a failed export left staging behind");
        std::fs::remove_dir_all(exports.parent().unwrap()).unwrap();
    }

    #[test]
    fn sweep_bounds_finished_exports_but_keeps_referenced_new_and_active_ones() {
        let exports = temp_exports("sweep");
        let day = Duration::from_secs(86_400);
        let referenced = finished(&exports, "aaaaaaaaaaaa", 30 * day);
        for (i, id) in ["old1", "old2", "old3", "old4", "old5"].iter().enumerate() {
            finished(&exports, id, (10 - i as u32) * day);
        }
        finished(&exports, "young", Duration::from_secs(5));
        // Crash leftovers: an unlocked partial with its lock, a partial with no lock,
        // and a lock whose partial never got created.
        std::fs::create_dir(exports.join("crash1.partial")).unwrap();
        std::fs::write(exports.join("crash1.partial/a.txt"), b"half").unwrap();
        std::fs::write(exports.join("crash1.lock"), b"").unwrap();
        std::fs::create_dir(exports.join("crash2.partial")).unwrap();
        std::fs::write(exports.join("crash3.lock"), b"").unwrap();
        // Concurrent export (this or another Itda process) still holding its lock.
        let held = hold_lock(&exports.join("busy.lock")).unwrap();
        std::fs::create_dir(exports.join("busy.partial")).unwrap();
        let busy_lock_only = hold_lock(&exports.join("starting.lock")).unwrap();
        // A non-directory entry is never touched.
        std::fs::write(exports.join("note"), b"keep").unwrap();

        sweep(&exports, &[referenced.join("a.txt")]);

        assert_eq!(
            names(&exports),
            ["aaaaaaaaaaaa", "busy.lock", "busy.partial", "note", "old4", "old5", "starting.lock", "young"]
        );
        drop((held, busy_lock_only));
        sweep(&exports, &[referenced.join("a.txt")]);
        assert_eq!(names(&exports), ["aaaaaaaaaaaa", "note", "old4", "old5", "young"]);
        // Nothing referenced: still only the newest few survive, paste-after-exit intact.
        sweep(&exports, &[]);
        assert_eq!(names(&exports), ["note", "old4", "old5", "young"]);
        std::fs::remove_dir_all(exports.parent().unwrap()).unwrap();
    }

    #[test]
    fn a_sweep_during_staging_keeps_the_in_progress_export() {
        let exports = temp_exports("concurrent");
        for id in ["o1", "o2", "o3", "o4"] {
            finished(&exports, id, Duration::from_secs(86_400));
        }
        let paths = stage(&exports, &[remote("a.txt")], |_, p| {
            sweep(&exports, &[]);
            assert!(p[0].parent().unwrap().is_dir(), "sweep removed a staging export");
            write_fixture(p)
        })
        .unwrap();
        assert_eq!(std::fs::read(&paths[0]).unwrap(), b"staged");
        sweep(&exports, &[]);
        assert!(paths[0].exists(), "the newest export was swept");
        std::fs::remove_dir_all(exports.parent().unwrap()).unwrap();
    }
}
