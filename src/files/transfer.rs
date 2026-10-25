//! Managed copy jobs. Copy and move never mirror/delete unrelated destination files.
use super::*;
use sha2::{Digest, Sha256};
use std::{
    fs::{File, OpenOptions},
    io::{Read, Write},
    process::{Command, Stdio},
    sync::atomic::AtomicUsize,
    time::{Duration, Instant},
};
static ACTIVE: AtomicUsize = AtomicUsize::new(0);
struct Slot;
impl Drop for Slot {
    fn drop(&mut self) {
        ACTIVE.fetch_sub(1, Ordering::SeqCst);
    }
}
fn slot(c: &Control) -> io::Result<Slot> {
    loop {
        c.check()?;
        let n = ACTIVE.load(Ordering::SeqCst);
        if n < 2
            && ACTIVE
                .compare_exchange(n, n + 1, Ordering::SeqCst, Ordering::SeqCst)
                .is_ok()
        {
            return Ok(Slot);
        }
        std::thread::sleep(Duration::from_millis(100));
    }
}
struct Endpoint {
    sftp: Option<Sftp>,
    /// To reach the remote again once a cancel has closed `sftp` with the job.
    cfg: Option<SshCfg>,
}
impl Endpoint {
    /// A device's files over its own SFTP session, or this computer's files (`None`).
    fn new(remote: Option<&Connection>, control: &Control) -> io::Result<Self> {
        Ok(Self {
            sftp: match remote {
                Some(c) => Some(Sftp::connect(&c.cfg, control)?),
                None => None,
            },
            cfg: remote.map(|c| c.cfg.clone()),
        })
    }
    /// Remove a staged copy that didn't get its name. A cancel stops this side's SFTP session
    /// with the job, so a remote leftover is removed through a new session; the server may
    /// still hold the file open for a moment after the old one closed.
    fn discard(&mut self, p: &str, cancelled: bool) {
        if self.remove(p, false).is_ok() || !cancelled {
            return;
        }
        let Some(cfg) = &self.cfg else { return };
        let control = Control::default();
        let Ok(mut s) = Sftp::connect(cfg, &control) else {
            return;
        };
        for _ in 0..10 {
            if s.remove(p, false).is_ok() || matches!(s.stat(p), Ok(None)) {
                return;
            }
            std::thread::sleep(Duration::from_millis(200));
        }
        crate::log!("files: staged copy left behind: {p}");
    }
    fn stat(&mut self, p: &str) -> io::Result<Option<Entry>> {
        if let Some(s) = &mut self.sftp {
            s.stat(p)
        } else {
            match local_entry(Path::new(p)) {
                Ok(e) => Ok(Some(e)),
                Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
                Err(e) => Err(e),
            }
        }
    }
    fn list(&mut self, p: &str) -> io::Result<Vec<Entry>> {
        if let Some(s) = &mut self.sftp {
            s.list(p)
        } else {
            local_list(p)
        }
    }
    fn mkdir(&mut self, p: &str) -> io::Result<()> {
        if let Some(s) = &mut self.sftp {
            s.mkdir(p)
        } else {
            std::fs::create_dir(p)
        }
    }
    fn remove(&mut self, p: &str, dir: bool) -> io::Result<()> {
        if let Some(s) = &mut self.sftp {
            s.remove(p, dir)
        } else if dir {
            std::fs::remove_dir(p)
        } else {
            std::fs::remove_file(p)
        }
    }
    fn rename(&mut self, from: &str, to: &str, replace: bool) -> io::Result<()> {
        if let Some(s) = &mut self.sftp {
            s.rename(from, to, replace)
        } else {
            rename_local(from, to, replace)
        }
    }
    /// Ordinary permission bits (& 0o777). Where unsupported (Windows) nothing happens.
    fn set_mode(&mut self, p: &str, mode: u32) -> io::Result<()> {
        if let Some(s) = &mut self.sftp {
            return s.set_mode(p, mode & 0o777);
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(p, std::fs::Permissions::from_mode(mode & 0o777))
        }
        #[cfg(not(unix))]
        {
            let _ = (p, mode);
            Ok(())
        }
    }
    /// Give a copy the source's permissions when there are some to give. A server that
    /// refuses (some Windows SFTP servers) leaves the safe default, not a failed copy.
    fn keep_mode(&mut self, p: &str, mode: Option<u32>) {
        if let Some(m) = mode {
            if let Err(e) = self.set_mode(p, m) {
                crate::log!("files: permissions not set on {p}: {e}");
            }
        }
    }
    fn hash(&mut self, p: &str, size: u64, c: &Control) -> io::Result<Vec<u8>> {
        struct Hash(Sha256);
        impl Write for Hash {
            fn write(&mut self, b: &[u8]) -> io::Result<usize> {
                self.0.update(b);
                Ok(b.len())
            }
            fn flush(&mut self) -> io::Result<()> {
                Ok(())
            }
        }
        let mut sink = Hash(Sha256::new());
        if let Some(s) = &mut self.sftp {
            s.download(p, size, &mut sink, c, |_| {})?;
        } else {
            let mut f = File::open(p)?;
            let mut buf = vec![0; 128 << 10];
            loop {
                c.check()?;
                let n = f.read(&mut buf)?;
                if n == 0 {
                    break;
                }
                sink.write_all(&buf[..n])?;
            }
        }
        Ok(sink.0.finalize().to_vec())
    }
}
struct Temp(PathBuf);
impl Drop for Temp {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}
fn unique_target(dst: &mut Endpoint, to: &str, dir: bool) -> io::Result<String> {
    let separator = to.rfind(['/', '\\']);
    let (parent, leaf) = separator.map_or(("", to), |i| (&to[..=i], &to[i + 1..]));
    let (stem, ext) = if dir {
        (leaf, "")
    } else {
        leaf.rfind('.')
            .filter(|&i| i > 0)
            .map_or((leaf, ""), |i| (&leaf[..i], &leaf[i..]))
    };
    for i in 2..=10_000 {
        let candidate = format!("{parent}{stem} ({i}){ext}");
        if dst.stat(&candidate)?.is_none() {
            return Ok(candidate);
        }
    }
    Err(error("사용 가능한 복사본 이름이 없습니다"))
}
struct Context {
    control: Arc<Control>,
    emit: Emit,
    id: String,
    job: String,
    policy: String,
    moving: bool,
    /// Carry permission bits over (not from a Windows source, which reports them loosely).
    modes: bool,
    /// Both ends are one filesystem (same device, or both this computer): a move may be a
    /// rename. Between two devices a rename would act on the source device alone.
    same_fs: bool,
    target_windows: bool,
    normalizing_tree: bool,
    done: u64,
    last: Instant,
}
impl Context {
    fn progress(&mut self, n: u64, total: u64, file: &str) {
        if self.last.elapsed() < Duration::from_millis(100) && n < total {
            return;
        }
        self.last = Instant::now();
        (self.emit)(
            json!({"type":"files_job","id":self.id,"job":self.job,"state":"running","file":file,"bytes":n,"total":total,"files":self.done,"engine":"sftp"}),
        );
    }
    fn copy(
        &mut self,
        src: &mut Endpoint,
        dst: &mut Endpoint,
        from: &str,
        to: &str,
        depth: usize,
    ) -> io::Result<()> {
        self.control.check()?;
        if depth > 128 {
            return Err(error("폴더가 너무 깊습니다"));
        }
        let before = src
            .stat(from)?
            .ok_or_else(|| error("원본 파일이 없습니다"))?;
        if before.link {
            return Err(error(format!(
                "심볼릭 링크는 자동으로 따라가지 않습니다: {}",
                before.name
            )));
        }
        let mut to = destination_leaf(to, self.target_windows);
        let mut existing = dst.stat(&to)?;
        if existing.is_some() && self.policy == "exclusive" {
            return Err(error("대상에 같은 이름의 파일이 있습니다"));
        }
        if existing.is_some() && self.policy == "skip" {
            return Ok(());
        }
        if existing.is_some() && self.policy == "keep" {
            to = unique_target(dst, &to, before.dir)?;
            existing = None;
        }
        if self.moving
            && self.same_fs
            && !(self.normalizing_tree && before.dir)
            && src.sftp.is_some() == dst.sftp.is_some()
            && existing.is_none()
            && src.rename(from, &to, false).is_ok()
        {
            self.done += 1;
            return Ok(());
        }
        // Cross-filesystem moves fall back to verified copy followed by removal.
        if before.dir {
            let created = existing.is_none();
            if let Some(e) = existing {
                if !e.dir {
                    return Err(error(format!(
                        "폴더와 파일 이름이 충돌합니다: {}",
                        before.name
                    )));
                }
            } else {
                dst.mkdir(&to)?;
            }
            for entry in src.list(from)? {
                self.copy(
                    src,
                    dst,
                    &join(from, &entry.name)?,
                    &join(&to, &entry.name)?,
                    depth + 1,
                )?;
            }
            // A folder made here gets the source's permissions once its contents are in (a
            // read-only one would refuse them); a folder merged into keeps its own.
            if created && self.modes {
                dst.keep_mode(&to, before.mode);
            }
            self.control.check()?;
            if self.moving {
                src.remove(from, true)?;
            }
            return Ok(());
        }
        if let Some(e) = existing {
            if e.dir {
                return Err(error(format!(
                    "파일과 폴더 이름이 충돌합니다: {}",
                    before.name
                )));
            }
            if !self.moving && e.size == before.size && e.modified == before.modified {
                self.done += 1;
                return Ok(());
            }
            if self.policy != "replace" {
                return Err(error(format!(
                    "같은 이름의 파일이 있습니다: {}",
                    before.name
                )));
            }
        }
        // Stage to a unique name in the destination directory. Cancellation never destroys
        // the old destination or removes the source. No --inplace writes.
        let temporary = format!(".itda-{}.partial", crate::store::new_id());
        let staging = if dst.sftp.is_some() {
            let parent = to.rsplit_once('/').map_or("", |p| p.0);
            join(parent, &temporary)?
        } else {
            Path::new(&to)
                .parent()
                .ok_or_else(|| error("대상 폴더가 없습니다"))?
                .join(temporary)
                .to_string_lossy()
                .into_owned()
        };
        let control = self.control.clone();
        let result = (|| -> io::Result<()> {
            if let Some(s) = &mut dst.sftp {
                if let Some(source) = &mut src.sftp {
                    if !self.same_fs {
                        source.relay_to(from, before.size, s, &staging, &control, |n| self.progress(n, before.size, &before.name))?;
                    } else {
                        let temp = Temp(
                            std::env::temp_dir().join(format!("itda-stage-{}", crate::store::new_id())),
                        );
                        let mut opts = OpenOptions::new();
                        opts.write(true).read(true).create_new(true);
                        #[cfg(unix)]
                        {
                            use std::os::unix::fs::OpenOptionsExt;
                            opts.mode(0o600);
                        }
                        let mut f = opts.open(&temp.0)?;
                        source.download(from, before.size, &mut f, &control, |n| {
                            self.progress(n, before.size, &before.name)
                        })?;
                        use std::io::Seek;
                        f.rewind()?;
                        s.upload(&staging, &mut f, &control, |n| {
                            self.progress(n, before.size, &before.name)
                        })?;
                    }
                } else {
                    let mut f = File::open(from)?;
                    s.upload(&staging, &mut f, &control, |n| {
                        self.progress(n, before.size, &before.name)
                    })?;
                }
                s.set_time(&staging, before.modified)?;
            } else {
                let mut opts = OpenOptions::new();
                opts.write(true).create_new(true);
                #[cfg(unix)]
                {
                    use std::os::unix::fs::OpenOptionsExt;
                    opts.mode(0o600);
                }
                let mut out = opts.open(&staging)?;
                if let Some(s) = &mut src.sftp {
                    s.download(from, before.size, &mut out, &control, |n| {
                        self.progress(n, before.size, &before.name)
                    })?;
                } else {
                    let mut f = File::open(from)?;
                    let mut buf = vec![0; 128 << 10];
                    let mut done = 0;
                    loop {
                        control.check()?;
                        let n = f.read(&mut buf)?;
                        if n == 0 {
                            break;
                        }
                        out.write_all(&buf[..n])?;
                        done += n as u64;
                        self.progress(done, before.size, &before.name);
                    }
                }
                out.sync_all()?;
                out.set_times(
                    std::fs::FileTimes::new()
                        .set_modified(UNIX_EPOCH + Duration::from_secs(before.modified)),
                )?;
            }
            let after = src
                .stat(from)?
                .ok_or_else(|| error("전송 중 원본이 사라졌습니다"))?;
            let staged = dst
                .stat(&staging)?
                .ok_or_else(|| error("전송 결과를 확인할 수 없습니다"))?;
            if before.size != after.size
                || before.modified != after.modified
                || staged.size != before.size
            {
                return Err(error("전송 중 파일이 변경되었습니다. 원본은 유지했습니다"));
            }
            if self.moving
                && src.hash(from, before.size, &control)?
                    != dst.hash(&staging, before.size, &control)?
            {
                return Err(error(
                    "복사 결과의 해시가 일치하지 않습니다. 원본은 유지했습니다",
                ));
            }
            control.check()?;
            let replace = dst.stat(&to)?.is_some();
            if replace && self.policy != "replace" {
                return Err(error(
                    "대상에 같은 이름의 파일이 생겼습니다. 다시 확인해 주세요",
                ));
            }
            // The staged copy stays 0600 until now; it takes the source's permissions just
            // before it gets its real name.
            if self.modes {
                dst.keep_mode(&staging, before.mode);
            }
            dst.rename(&staging, &to, replace)?;
            if self.moving {
                control.check()?;
                let latest = src
                    .stat(from)?
                    .ok_or_else(|| error("원본이 변경되었습니다"))?;
                if latest.size != before.size || latest.modified != before.modified {
                    return Err(error("원본이 변경되어 삭제하지 않았습니다"));
                }
                src.remove(from, false)?;
            }
            self.done += 1;
            Ok(())
        })();
        if result.is_err() {
            dst.discard(&staging, self.control.cancel.load(Ordering::Relaxed));
        }
        result
    }
}
#[cfg(target_os = "macos")]
pub(super) fn export_batch(files: &[super::export::File], targets: &[PathBuf], emit: Emit) -> io::Result<()> {
    let file=files.first().ok_or_else(||error("파일을 선택하세요"))?;
    if files.len()!=targets.len() || files.iter().any(|f|f.id!=file.id||f.remote!=file.remote){return Err(error("복사할 파일 위치를 확인하세요"));}
    let control=Arc::new(Control::default());
    // A Finder promise's write: its queue counts it (and keeps counting after this job).
    control.promise.store(super::PROMISE_WRITE.with(|p| p.get()), Ordering::Relaxed);
    let job=crate::store::new_id();
    if !super::admit(&job, &file.id, &control) { return Err(error(super::CLOSING)); }
    emit(json!({"type":"files_job","id":file.id,"job":job,"state":"queued","kind":"export","file":file.name}));
    let result: io::Result<()>=(|| {
        let _slot=slot(&control)?;
        let mut src=if file.remote { Endpoint::new(Some(&*connection(&file.id)?),&control)? } else { Endpoint{sftp:None,cfg:None} };
        let mut dst=Endpoint{sftp:None,cfg:None};
        let mut cx=Context{control:control.clone(),emit:emit.clone(),id:file.id.clone(),job:job.clone(),
            policy:"exclusive".into(),moving:false,modes:true,same_fs:false,target_windows:false,normalizing_tree:false,done:0,last:Instant::now()-Duration::from_secs(1)};
        for(file,target)in files.iter().zip(targets){cx.copy(&mut src,&mut dst,&file.path()?.to_string_lossy(),&target.to_string_lossy(),0)?;}
        Ok(())
    })();
    jobs().lock().unwrap().remove(&job);
    super::cleanup_retired(&file.id);
    emit(json!({"type":"files_job","id":file.id,"job":job,"state":if result.is_ok(){"completed"}else if control.cancel.load(Ordering::Relaxed){"cancelled"}else{"failed"},"error":result.as_ref().err().map(|e|e.to_string())}));
    result
}
pub fn start(v: Value, host: Host, emit: Emit) {
    let id = host.id.clone();
    let key = super::scoped_id(&id, &v["window"]);
    // The page's own handle for this request, so its job can be matched to what was asked
    // (for a retry of what is left). Not a secret, and never a path or a token.
    let req: String = v["req"].as_str().unwrap_or("").chars().filter(|c| c.is_ascii_alphanumeric()).take(64).collect();
    // Another device's files (another tab, or another file window's page) as the source; the
    // job belongs to the destination device, where it shows and can be cancelled.
    let source_key = v["from_id"].as_str().filter(|f| !f.is_empty()).map(|f| {
        let window = if v["from_window"].is_string() { &v["from_window"] } else { &v["window"] };
        super::scoped_id(f, window)
    });
    // One device even when the copy came from another window of it: then the guards against
    // copying a folder into itself and the rename-for-move apply as usual.
    let same_device = v["from_id"].as_str().is_none_or(|f| f.is_empty() || f == id);
    // Capture the source connection before the job queues: closing its window later must
    // not invalidate a copy already accepted by the destination window.
    let held_source = if v["from"] == "remote" && !same_device {
        source_key.as_ref().map(|k| super::transfer_source(k, v["from_clip"].as_str(), v["from_drag"].as_str())).transpose()
    } else { Ok(None) };
    let source_closed = held_source.is_err();
    let job = crate::store::new_id();
    let control = Arc::new(Control::default());
    if !super::admit(&job, &key, &control) {
        emit(json!({"type":"files_job","id":id,"job":job,"req":req,"state":"failed","error":super::CLOSING,"completed_names":[]}));
        return;
    }
    {
        let mut q = jobs().lock().unwrap();
        if q.len() > 32 {
            q.remove(&job);
            emit(
                json!({"type":"files_job","id":id,"job":job,"req":req,"state":"failed","error":"대기 중인 전송이 많습니다. 일부 작업이 끝난 뒤 다시 시도하세요","completed_names":[]}),
            );
            return;
        }
    }
    emit(json!({"type":"files_job","id":id,"job":job,"req":req,"state":"queued","moving":v["move"]}));
    std::thread::spawn(move || {
        // Top-level names finished (copied, moved, or skipped by the policy) and the one being
        // worked on: after a failure or cancel, a retry offers only what is left.
        super::preview_cache::invalidate_all();
        let mut completed: Vec<String> = Vec::new();
        let mut partial: Option<(String, bool)> = None;
        let result = (|| -> io::Result<()> {
            let _slot = slot(&control)?;
            let from_remote = v["from"] == "remote";
            let to_remote = v["to"] == "remote";
            if !(v["from"] == "remote" || v["from"] == "local")
                || !(v["to"] == "remote" || v["to"] == "local")
            {
                return Err(error("전송 위치가 올바르지 않습니다"));
            }
            // Only the remote ends need a device connection: this computer's own files are
            // copied without one (a file window with no device yet).
            let dst_c = if to_remote || (from_remote && same_device) { Some(connection(&key)?) } else { None };
            let src_c = match (&source_key, from_remote) {
                (_, false) => None,
                (Some(_), true) if !same_device => Some(held_source.map_err(|_| error("원본 기기의 파일 연결이 닫혔습니다. 그 기기의 탭이나 창을 다시 열어 주세요"))?.ok_or_else(|| error("원본 기기의 파일 연결을 확인하세요"))?),
                _ => dst_c.clone(),
            };
            let (src_c, dst_c) = (src_c.filter(|_| from_remote), dst_c.filter(|_| to_remote));
            let mut src_path = get(&v, "source");
            let mut dst_path = get(&v, "destination");
            if let Some(c) = &src_c {
                src_path = remote_path(&src_path, &c.home);
            }
            if let Some(c) = &dst_c {
                dst_path = remote_path(&dst_path, &c.home);
            }
            if src_path.is_empty() || dst_path.is_empty() {
                return Err(error("전송할 위치를 확인하세요"));
            }
            let src_windows = src_c.as_ref().map_or(cfg!(windows), |c| c.windows);
            let dst_windows = dst_c.as_ref().map_or(cfg!(windows), |c| c.windows);
            // The one remote end rsync can reach (it runs between this computer and a device).
            let rsync_end = src_c.as_ref().or(dst_c.as_ref()).cloned();
            let moving = v["move"] == true;
            let policy = get(&v, "policy");
            if !["skip", "keep", "replace"].contains(&policy.as_str()) {
                return Err(error("이름 충돌 처리 방법을 선택하세요"));
            }
            let names = v["names"]
                .as_array()
                .ok_or_else(|| error("파일을 선택하세요"))?;
            if names.is_empty() || names.len() > 10_000 {
                return Err(error("전송할 파일 수를 확인하세요"));
            }
            let mut src = Endpoint::new(src_c.as_deref(), &control)?;
            let mut dst = Endpoint::new(dst_c.as_deref(), &control)?;
            // Two saved devices can be one machine (a LAN and a Tailscale entry): a folder made
            // through the source and seen through the destination means one filesystem, where a
            // copy into itself is refused and replacing "the other" copy would delete the only one.
            let mut same_fs = same_device || (!from_remote && !to_remote);
            if !same_fs && from_remote && to_remote {
                if let Ok(real) = src.sftp.as_mut().map_or(Ok(src_path.clone()), |s| s.realpath(&src_path)) {
                    let marker = join(&real, &format!(".itda-same-{}", crate::store::new_id()))?;
                    if src.mkdir(&marker).is_ok() {
                        same_fs = matches!(dst.stat(&marker), Ok(Some(_)));
                        let _ = src.remove(&marker, true);
                    }
                }
            }
            if from_remote == to_remote {
                src_path = if let Some(s) = &mut src.sftp {
                    s.realpath(&src_path)?
                } else {
                    super::canonical(&src_path)?
                };
                dst_path = if let Some(s) = &mut dst.sftp {
                    s.realpath(&dst_path)?
                } else {
                    super::canonical(&dst_path)?
                };
            }
            let mut cx = Context {
                control: control.clone(),
                emit: emit.clone(),
                id: id.clone(),
                job: job.clone(),
                policy,
                moving,
                modes: !(from_remote && src_windows),
                same_fs,
                target_windows: dst_windows,
                normalizing_tree: false,
                done: 0,
                last: Instant::now() - Duration::from_secs(1),
            };
            // Probe rsync once per job. Failure to offer it falls back to SFTP, before any data
            // changes. Explicit delta mode reports the missing capability instead.
            let requested = get(&v, "engine");
            let mut delta = None;
            // Every name written to a Windows side is checked before anything is created, so
            // a refused name deep in a folder leaves no half-copied folder behind.
            let mut normalizing = std::collections::HashSet::new();
            if dst_windows {
                let mut unique = std::collections::HashSet::new();
                for n in names {
                    let n = n.as_str().ok_or_else(|| error("잘못된 파일 이름"))?;
                    name(n)?;
                    windows_target(n, n)?;
                    if !unique.insert(windows_name_key(n)) { return Err(error("Windows에서 같은 이름이 되는 파일이 여러 개 있습니다. 이름을 바꾼 뒤 복사하세요")); }
                    // rsync can rename the top-level argument explicitly; only names
                    // inside a folder need the component-by-component SFTP path.
                    let mut changes = false;
                    let from = if from_remote {
                        join(&src_path, n)?
                    } else {
                        Path::new(&src_path).join(n).to_string_lossy().into_owned()
                    };
                    if src.stat(&from)?.is_some_and(|e| e.dir && !e.link) {
                        changes |= windows_tree(&mut src, &from, n, 0, &control)?;
                    }
                    if changes { normalizing.insert(n.to_string()); }
                }
            }
            for n in names {
                control.check()?;
                let n = n.as_str().ok_or_else(|| error("잘못된 파일 이름"))?;
                name(n)?;
                let from = if from_remote {
                    join(&src_path, n)?
                } else {
                    Path::new(&src_path).join(n).to_string_lossy().into_owned()
                };
                cx.normalizing_tree = normalizing.contains(n);
                let to_name = destination_name(n,dst_windows);
                let to = if to_remote {
                    join(&dst_path, &to_name)?
                } else {
                    Path::new(&dst_path).join(&to_name).to_string_lossy().into_owned()
                };
                let compare = |p: &str| {
                    let p = p.replace('\\', "/");
                    if src_windows {
                        windows_name_key(&p)
                    } else {
                        p
                    }
                };
                let (f, t) = (compare(&from), compare(&to));
                if from_remote == to_remote
                    && same_fs
                    && (f == t || t.starts_with(&format!("{}/", f.trim_end_matches('/'))))
                {
                    return Err(error(
                        "원본과 같은 위치 또는 원본 폴더 안으로 복사할 수 없습니다",
                    ));
                }
                let entry = src
                    .stat(&from)?
                    .ok_or_else(|| error("원본 파일이 없습니다"))?;
                partial = Some((n.to_string(), entry.dir));
                let destination = dst.stat(&to)?;
                if destination
                    .as_ref()
                    .is_some_and(|d| d.dir != entry.dir || d.link)
                {
                    return Err(error(
                        "파일/폴더 종류가 충돌하거나 대상이 심볼릭 링크입니다",
                    ));
                }
                if requested == "rsync" && cx.normalizing_tree {
                    return Err(error("Windows에 한글 파일명을 완성형으로 저장하려면 자동 또는 일반 전송을 선택하세요"));
                }
                let candidate = !cx.normalizing_tree && (requested == "rsync" || entry.dir || destination.is_some())
                    && requested != "sftp"
                    && from_remote != to_remote
                    && !moving
                    && cx.policy == "replace"
                    && !entry.link
                    && (requested == "rsync" || entry.dir || entry.size >= 8 << 20);
                let can_delta = if candidate {
                    *delta.get_or_insert_with(|| rsync_end.as_ref().is_some_and(|c| rsync_available() && probe_rsync(&c.cfg, &control)))
                } else {
                    false
                };
                if requested == "rsync" && !can_delta {
                    return Err(error(
                        "이 작업은 rsync를 사용할 수 없습니다. 자동 또는 일반 전송을 선택하세요",
                    ));
                }
                if can_delta {
                    emit(
                        json!({"type":"files_job","id":id,"job":job,"state":"running","file":n,"engine":"rsync","detail":"변경된 파일과 블록을 비교하고 있습니다"}),
                    );
                    let rc = rsync_end.as_ref().ok_or_else(|| error("rsync를 사용할 수 없습니다"))?;
                    match run_rsync(&rc.cfg, &from, &to, from_remote, entry.dir, &control) {
                        Ok(()) => cx.done += 1,
                        // Asked for rsync: say why it failed. Chosen automatically: an rsync
                        // that won't run here (version, paths) shouldn't fail the copy, so
                        // the ordinary transfer does it ("replace" policy, nothing moved).
                        Err(e) if requested == "rsync" || control.cancel.load(Ordering::Relaxed) => return Err(e),
                        Err(e) => {
                            crate::log!("files: rsync failed, copying over SFTP instead: {e}");
                            emit(
                                json!({"type":"files_job","id":id,"job":job,"state":"running","file":n,"engine":"sftp","detail":"rsync를 쓸 수 없어 일반 전송으로 복사합니다"}),
                            );
                            cx.copy(&mut src, &mut dst, &from, &to, 0)?;
                        }
                    }
                } else {
                    cx.copy(&mut src, &mut dst, &from, &to, 0)?;
                }
                completed.push(n.to_string());
                partial = None;
            }
            Ok(())
        })();
        jobs().lock().unwrap().remove(&job);
        super::preview_cache::invalidate_all();
        super::cleanup_retired(&key);
        let cancelled = control.cancel.load(Ordering::Relaxed);
        // A cancel ends the job's SSH sessions, so the error may only be their broken pipe.
        let error = result.err().map(|e| {
            if cancelled {
                "전송을 취소했습니다".to_string()
            } else {
                e.to_string()
            }
        });
        let partial = partial.filter(|_| error.is_some()).map(|(name, dir)| json!({"name":name,"dir":dir}));
        emit(
            json!({"type":"files_job","id":id,"job":job,"req":req,"state":if error.is_none(){"completed"}else if cancelled{"cancelled"}else{"failed"},"error":error,
                "completed_names":completed,"partial":partial,"source_closed":source_closed&&error.is_some()}),
        );
    });
}
/// Every name under a source folder, checked against what Windows can store.
fn windows_tree(src: &mut Endpoint, path: &str, shown: &str, depth: usize, c: &Control) -> io::Result<bool> {
    c.check()?;
    if depth > 128 {
        return Err(error("폴더가 너무 깊습니다"));
    }
    let mut changes = false;
    let mut unique = std::collections::HashSet::new();
    for e in src.list(path)? {
        let shown = format!("{shown}/{}", e.name);
        windows_target(&e.name, &shown)?;
        if !unique.insert(windows_name_key(&e.name)) { return Err(error(format!("Windows에서 같은 이름이 되는 파일이 여러 개 있습니다: {shown}"))); }
        changes |= destination_name(&e.name,true) != e.name;
        if e.dir && !e.link {
            changes |= windows_tree(src, &join(path, &e.name)?, &shown, depth + 1, c)?;
        }
    }
    Ok(changes)
}
fn rsync_exe() -> Option<PathBuf> {
    let name = if cfg!(windows) { "rsync.exe" } else { "rsync" };
    std::env::split_paths(&std::env::var_os("PATH")?)
        .map(|d| d.join(name))
        .find(|p| p.is_file())
}
pub fn rsync_available() -> bool {
    #[cfg(windows)]
    {
        return rsync_exe()
            .is_some_and(|p| p.parent().is_some_and(|p| p.join("cygpath.exe").is_file()));
    }
    #[cfg(not(windows))]
    {
        rsync_exe().is_some()
    }
}
fn rsync_local_path(path: &str) -> io::Result<String> {
    #[cfg(windows)]
    {
        let exe = rsync_exe()
            .and_then(|p| p.parent().map(|p| p.join("cygpath.exe")))
            .ok_or_else(|| error("호환되는 rsync 경로 변환 도구가 없습니다"))?;
        let out = Command::new(exe).args(["-u", "--", path]).output()?;
        if !out.status.success() {
            return Err(error("rsync 파일 경로를 변환할 수 없습니다"));
        }
        let value =
            String::from_utf8(out.stdout).map_err(|_| error("rsync 경로 인코딩을 확인하세요"))?;
        Ok(value.trim_end_matches(['\r', '\n']).to_string())
    }
    #[cfg(not(windows))]
    {
        Ok(path.to_string())
    }
}
/// Quote for the remote (POSIX) shell.
fn quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}

/// Quote one word of rsync's -e command. rsync splits it itself, honouring '...' and "..."
/// but not the shell's '\'' escape, so a word with ' goes in double quotes instead.
fn rsync_word(s: &str) -> io::Result<String> {
    if !s.contains('\'') {
        Ok(format!("'{s}'"))
    } else if !s.contains('"') && !s.contains('\\') {
        Ok(format!("\"{s}\""))
    } else {
        Err(error("rsync에 넘길 수 없는 경로입니다 (따옴표). 일반 전송을 선택하세요"))
    }
}
fn run(mut cmd: Command, c: &Control, timeout: Option<Duration>) -> io::Result<()> {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x08000000);
    }
    let mut child = cmd
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()?;
    crate::tunnel::register_child(&child);
    let pipe = child.stderr.take().unwrap();
    let log = Arc::new(Mutex::new(String::new()));
    let l = log.clone();
    let logger = std::thread::spawn(move || {
        use std::io::BufRead;
        for line in std::io::BufReader::new(pipe).lines().map_while(Result::ok) {
            let mut l = l.lock().unwrap();
            l.push_str(&line);
            l.push('\n');
            if l.len() > 8192 {
                let at = l
                    .char_indices()
                    .find(|(i, _)| *i >= l.len() - 4096)
                    .map(|(i, _)| i)
                    .unwrap_or(0);
                l.drain(..at);
            }
        }
    });
    let child = Arc::new(Mutex::new(child));
    c.attach(child.clone())?;
    let start = Instant::now();
    loop {
        if let Err(e) = c.check() {
            let mut ch = child.lock().unwrap();
            let _ = ch.kill();
            let _ = ch.wait();
            return Err(e);
        }
        if timeout.is_some_and(|d| start.elapsed() > d) {
            let mut ch = child.lock().unwrap();
            let _ = ch.kill();
            let _ = ch.wait();
            return Err(error("SSH 기능 확인 시간 초과"));
        }
        if let Some(status) = child.lock().unwrap().try_wait()? {
            let _ = logger.join();
            return if status.success() {
                Ok(())
            } else {
                Err(error(format!(
                    "rsync 전송 실패: {}",
                    log.lock().unwrap().trim()
                )))
            };
        }
        std::thread::sleep(Duration::from_millis(100));
    }
}
fn probe_rsync(cfg: &SshCfg, c: &Control) -> bool {
    let Ok(mut cmd) = crate::tunnel::ssh_command(cfg) else {
        return false;
    };
    let Ok(target) = crate::tunnel::ssh_target(cfg) else {
        return false;
    };
    cmd.arg(target)
        .arg(r#"test -n "$HOME" && command -v rsync >/dev/null && rsync --version"#);
    run(cmd, c, Some(Duration::from_secs(12))).is_ok()
}
pub(super) fn run_rsync(
    cfg: &SshCfg,
    from: &str,
    to: &str,
    download: bool,
    dir: bool,
    c: &Control,
) -> io::Result<()> {
    run(rsync_command(cfg, from, to, download, dir)?, c, None)
}
fn rsync_command(cfg: &SshCfg, from: &str, to: &str, download: bool, dir: bool) -> io::Result<Command> {
    let mut ssh = crate::tunnel::ssh_command(cfg)?;
    ssh.args(["-o", "ControlMaster=no", "-o", "ControlPath=none"]);
    let mut shell = rsync_word(&ssh.get_program().to_string_lossy())?;
    for arg in ssh.get_args() {
        shell.push(' ');
        shell.push_str(&rsync_word(&arg.to_string_lossy())?);
    }
    // The account is in the ssh command (-l), so names with '@' work here too.
    let target = crate::tunnel::ssh_target(cfg)?;
    let target = if target.contains(':') { format!("[{target}]") } else { target };
    let suffix = if dir { "/" } else { "" };
    let remote = |p: &str| format!("{target}:{}", quote(&format!("{p}{suffix}")));
    let mut cmd = Command::new(rsync_exe().ok_or_else(|| error("rsync를 찾을 수 없습니다"))?);
    for (k, v) in ssh.get_envs() {
        if let Some(v) = v {
            cmd.env(k, v);
        }
    }
    // rsync 3.2.4+ protects remote paths itself; the paths above are already quoted for the
    // remote shell, so ask for the old behaviour, also over an inherited RSYNC_PROTECT_ARGS
    // (as rsync's manual advises for manual quoting; openrsync ignores both).
    cmd.env("RSYNC_OLD_ARGS", "1").env("RSYNC_PROTECT_ARGS", "0");
    cmd.args(["-rlt", "--partial-dir=.itda-partial", "--stats", "-e"])
        .arg(shell)
        .arg("--");
    if download {
        cmd.arg(remote(from))
            .arg(format!("{}{suffix}", rsync_local_path(to)?));
    } else {
        cmd.arg(format!("{}{suffix}", rsync_local_path(from)?))
            .arg(remote(to));
    }
    Ok(cmd)
}
#[cfg(test)]
mod tests {
    use super::*;
    fn fixture() -> (PathBuf, Endpoint, Endpoint, Context) {
        let root = std::env::temp_dir().join(format!("itda-copy-test-{}", crate::store::new_id()));
        std::fs::create_dir_all(root.join("from")).unwrap();
        std::fs::create_dir_all(root.join("to")).unwrap();
        let cx = Context {
            control: Arc::new(Control::default()),
            emit: Arc::new(|_| {}),
            id: "test".into(),
            job: "test".into(),
            policy: "keep".into(),
            moving: false,
            modes: true,
            same_fs: true,
            target_windows: false,
            normalizing_tree: false,
            done: 0,
            last: Instant::now() - Duration::from_secs(1),
        };
        (root, Endpoint { sftp: None, cfg: None }, Endpoint { sftp: None, cfg: None }, cx)
    }

    #[test]
    fn windows_destinations_compose_names_and_keep_source_bytes() {
        let nfd="정보공개 결정통지서(접수번호_12345678).pdf";
        let nfc="정보공개 결정통지서(접수번호_12345678).pdf";
        assert_eq!(destination_name(nfd,true),nfc);
        assert_eq!(destination_name(nfd,false),nfd);
        assert_eq!(windows_name_key(nfd),windows_name_key(nfc));
        assert_eq!(destination_leaf(&format!("/existing/가/{nfd}"),true),format!("/existing/가/{nfc}"));
        let (root,mut src,mut dst,mut cx)=fixture();
        let from=root.join("from"); let to=root.join("to");
        std::fs::create_dir(from.join("폴더")).unwrap();
        std::fs::write(from.join(nfd),b"%PDF-1.4 synthetic bytes\n").unwrap();
        std::fs::write(from.join("폴더/한글.txt"),b"nested bytes").unwrap();
        assert!(windows_tree(&mut src,&from.to_string_lossy(),"source",0,&cx.control).unwrap());
        cx.target_windows=true; cx.normalizing_tree=true; cx.policy="replace".into();
        cx.copy(&mut src,&mut dst,&from.to_string_lossy(),&to.to_string_lossy(),0).unwrap();
        assert_eq!(std::fs::read(to.join(nfc)).unwrap(),b"%PDF-1.4 synthetic bytes\n");
        assert_eq!(std::fs::read(to.join("폴더/한글.txt")).unwrap(),b"nested bytes");
        assert_eq!(std::fs::read(from.join(nfd)).unwrap(),b"%PDF-1.4 synthetic bytes\n");
        #[cfg(windows)]
        for entry in std::fs::read_dir(&to).unwrap() { let name=entry.unwrap().file_name().to_string_lossy().into_owned(); assert_eq!(destination_name(&name,true),name); }
        // Existing normalized names still use the normal conflict policy.
        cx.policy="keep".into();
        cx.copy(&mut src,&mut dst,&from.join(nfd).to_string_lossy(),&to.join(nfd).to_string_lossy(),0).unwrap();
        assert_eq!(std::fs::read_dir(&to).unwrap().count(),3);
        std::fs::remove_dir_all(root).unwrap();
    }
    #[cfg(windows)]
    #[test]
    fn normalization_aliases_are_rejected_before_writing() {
        let (root,mut src,_,cx)=fixture();
        std::fs::write(root.join("from/가.txt"),b"composed").unwrap();
        std::fs::write(root.join("from/가.txt"),b"decomposed").unwrap();
        assert_eq!(std::fs::read_dir(root.join("from")).unwrap().count(),2);
        assert!(windows_tree(&mut src,&root.join("from").to_string_lossy(),"source",0,&cx.control).is_err());
        assert_eq!(std::fs::read_dir(root.join("to")).unwrap().count(),0);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn remote_relay_keeps_transaction_and_move_guards() {
        let server = std::env::var_os("ITDA_SFTP_SERVER").map(PathBuf::from).unwrap_or_else(|| "/usr/libexec/sftp-server".into());
        if !server.is_file() { return; }
        let (root, mut src, mut dst, mut cx) = fixture();
        let open = |c: &Control| Sftp::start(Command::new(&server).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::null()).spawn().unwrap(), c).unwrap();
        src.sftp = Some(open(&cx.control)); dst.sftp = Some(open(&cx.control));
        cx.same_fs = false; cx.policy = "replace".into();
        let from = root.join("from/한글.bin").to_string_lossy().into_owned();
        let to = root.join("to/한글.bin").to_string_lossy().into_owned();
        let data: Vec<u8> = (0..(3 * 1024 * 1024 + 7)).map(|i| (i % 239) as u8).collect();
        std::fs::write(&from, &data).unwrap();std::fs::write(&to, b"old target").unwrap();
        let events = Arc::new(Mutex::new(Vec::new()));let seen = events.clone();
        cx.emit = Arc::new(move |v| seen.lock().unwrap().push(v));
        cx.copy(&mut src, &mut dst, &from, &to, 0).unwrap();
        assert_eq!(std::fs::read(&to).unwrap(), data);assert_eq!(std::fs::read(&from).unwrap(), data);
        let steps:Vec<u64> = events.lock().unwrap().iter().filter_map(|v|v["bytes"].as_u64()).collect();
        assert!(steps.windows(2).all(|n|n[0]<=n[1]));assert!(steps.iter().any(|&n|n>0&&n<data.len()as u64));
        // A cross-device move runs the same stream, verifies hashes, then removes the source.
        cx.moving = true;
        cx.copy(&mut src, &mut dst, &from, &to, 0).unwrap();
        assert!(!Path::new(&from).exists());assert_eq!(std::fs::read(&to).unwrap(), data);
        drop((src,dst));
        // Cancellation during replacement leaves both the existing destination and source.
        std::fs::write(&from,&data).unwrap();std::fs::write(&to,b"keep existing").unwrap();
        cx.control = Arc::new(Control::default());cx.moving=true;
        let stop=cx.control.clone();cx.emit=Arc::new(move|v|{if v["bytes"].as_u64().is_some_and(|n|n>0){stop.stop();}});
        let mut src=Endpoint{sftp:Some(open(&cx.control)),cfg:None};let mut dst=Endpoint{sftp:Some(open(&cx.control)),cfg:None};
        assert!(cx.copy(&mut src,&mut dst,&from,&to,0).is_err());
        assert_eq!(std::fs::read(&to).unwrap(),b"keep existing");assert_eq!(std::fs::read(&from).unwrap(),data);
        drop((src,dst));std::fs::remove_dir_all(root).unwrap();
    }

    /// Between two devices a move must be a verified copy and delete, never a rename (which
    /// would act on the source device alone). Seen here as a new file, not the same inode.
    #[cfg(unix)]
    #[test]
    fn a_move_between_devices_copies_instead_of_renaming() {
        use std::os::unix::fs::MetadataExt;
        for same_fs in [true, false] {
            let (root, mut src, mut dst, mut cx) = fixture();
            cx.moving = true;
            cx.same_fs = same_fs;
            let (from, to) = (root.join("from/a.txt"), root.join("to/a.txt"));
            std::fs::write(&from, b"payload").unwrap();
            let inode = std::fs::metadata(&from).unwrap().ino();
            cx.copy(&mut src, &mut dst, &from.to_string_lossy(), &to.to_string_lossy(), 0).unwrap();
            assert!(!from.exists() && std::fs::read(&to).unwrap() == b"payload");
            assert_eq!(std::fs::metadata(&to).unwrap().ino() == inode, same_fs, "same_fs={same_fs}");
            std::fs::remove_dir_all(root).unwrap();
        }
    }
    #[test]
    fn copy_conflicts_folders_and_move() {
        let (r, mut a, mut b, mut cx) = fixture();
        let from = r.join("from/report.txt");
        let to = r.join("to/report.txt");
        std::fs::write(&from, b"new report").unwrap();
        std::fs::write(&to, b"old").unwrap();
        cx.copy(
            &mut a,
            &mut b,
            from.to_str().unwrap(),
            to.to_str().unwrap(),
            0,
        )
        .unwrap();
        assert_eq!(std::fs::read(&to).unwrap(), b"old");
        assert_eq!(
            std::fs::read(r.join("to/report (2).txt")).unwrap(),
            b"new report"
        );
        cx.policy = "skip".into();
        cx.copy(
            &mut a,
            &mut b,
            from.to_str().unwrap(),
            to.to_str().unwrap(),
            0,
        )
        .unwrap();
        assert_eq!(std::fs::read(&to).unwrap(), b"old");
        cx.policy = "replace".into();
        cx.copy(
            &mut a,
            &mut b,
            from.to_str().unwrap(),
            to.to_str().unwrap(),
            0,
        )
        .unwrap();
        assert_eq!(std::fs::read(&to).unwrap(), b"new report");
        std::fs::create_dir(r.join("from/Folder")).unwrap();
        std::fs::write(r.join("from/Folder/nested"), b"nested").unwrap();
        std::fs::create_dir(r.join("to/Folder")).unwrap();
        cx.policy = "keep".into();
        cx.copy(
            &mut a,
            &mut b,
            r.join("from/Folder").to_str().unwrap(),
            r.join("to/Folder").to_str().unwrap(),
            0,
        )
        .unwrap();
        assert_eq!(
            std::fs::read(r.join("to/Folder (2)/nested")).unwrap(),
            b"nested"
        );
        cx.moving = true;
        let moved = r.join("to/moved.txt");
        cx.copy(
            &mut a,
            &mut b,
            from.to_str().unwrap(),
            moved.to_str().unwrap(),
            0,
        )
        .unwrap();
        assert!(!from.exists());
        assert_eq!(std::fs::read(moved).unwrap(), b"new report");
        std::fs::remove_dir_all(r).unwrap();
    }
    #[test]
    fn cancellation_keeps_source_and_existing_destination() {
        let (r, mut a, mut b, mut cx) = fixture();
        let from = r.join("from/big.bin");
        let to = r.join("to/big.bin");
        std::fs::write(&from, vec![17; 4 << 20]).unwrap();
        std::fs::write(&to, b"original").unwrap();
        cx.policy = "replace".into();
        let c = cx.control.clone();
        cx.emit = Arc::new(move |_| c.stop());
        assert!(cx
            .copy(
                &mut a,
                &mut b,
                from.to_str().unwrap(),
                to.to_str().unwrap(),
                0
            )
            .is_err());
        assert_eq!(std::fs::read(&to).unwrap(), b"original");
        assert_eq!(std::fs::metadata(from).unwrap().len(), 4 << 20);
        assert_eq!(std::fs::read_dir(r.join("to")).unwrap().count(), 1);
        std::fs::remove_dir_all(r).unwrap();
    }
    /// The job's end says which top-level names finished and which one was cut off, for a
    /// retry of what is left; the page's request handle comes back with it.
    #[test]
    #[cfg(unix)]
    fn a_failed_job_reports_finished_and_partial_names() {
        let root = std::env::temp_dir().join(format!("itda-job-test-{}", crate::store::new_id()));
        let (from, to) = (root.join("from"), root.join("to"));
        std::fs::create_dir_all(from.join("dir")).unwrap();
        std::fs::create_dir_all(&to).unwrap();
        std::fs::write(from.join("a.txt"), b"a").unwrap();
        std::fs::write(from.join("dir/x.txt"), b"x").unwrap();
        std::os::unix::fs::symlink("x.txt", from.join("dir/link")).unwrap();
        std::fs::write(from.join("b.txt"), b"b").unwrap();
        let run = |names: Value| {
            let (tx, rx) = std::sync::mpsc::channel();
            let tx = Mutex::new(tx);
            let v = json!({"from":"local","to":"local","source":from.to_str().unwrap(),"destination":to.to_str().unwrap(),
                "names":names,"policy":"keep","move":false,"engine":"sftp","req":"r1/../x"});
            start(v, Host { id: crate::store::new_id(), ..Default::default() }, Arc::new(move |e: Value| { let _ = tx.lock().unwrap().send(e); }));
            loop {
                let e = rx.recv_timeout(Duration::from_secs(20)).unwrap();
                let state = e["state"].as_str().unwrap_or("");
                if state != "running" { assert_eq!(e["req"], "r1x", "{e}"); }
                if ["completed", "failed", "cancelled"].contains(&state) { return e; }
            }
        };
        let e = run(json!(["a.txt", "missing.txt", "b.txt"]));
        assert_eq!(e["state"], "failed");
        assert_eq!(e["completed_names"], json!(["a.txt"]));
        assert!(e["partial"].is_null() && e["source_closed"] == false);
        let e = run(json!(["b.txt", "dir"]));
        assert_eq!(e["state"], "failed");
        assert_eq!(e["completed_names"], json!(["b.txt"]));
        assert_eq!(e["partial"], json!({"name":"dir","dir":true}));
        std::fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn shell_quote() {
        assert_eq!(quote("a'b c"), "'a'\\''b c'");
    }
    #[test]
    fn cancellation_is_immediate() {
        let c = Control::default();
        c.stop();
        assert_eq!(c.check().unwrap_err().kind(), io::ErrorKind::Interrupted);
    }
    /// rsync's -e words: plain ones in '...', ones with ' in "...", never the shell's '\''.
    #[test]
    fn rsync_words() {
        assert_eq!(rsync_word("/usr/bin/ssh").unwrap(), "'/usr/bin/ssh'");
        assert_eq!(rsync_word("/Users/O'Brien/.ssh/id").unwrap(), "\"/Users/O'Brien/.ssh/id\"");
        assert!(rsync_word("a'b\"c").is_err());
        assert!(rsync_word(r"C:\Users\O'Brien\key").is_err());
    }

    /// The rsync command: old-args mode, the account in the ssh command (an '@' in it is
    /// fine), and the remote side as host:'path' without the account.
    #[test]
    fn rsync_command_line() {
        if rsync_exe().is_none() {
            return;
        }
        let cfg = SshCfg { host: "pc.local".into(), port: 22, user: "me@outlook.com".into(), pass: String::new(), vnc_port: 5900, key: String::new(), key_pass: String::new(), key_alias: String::new() };
        let Ok(cmd) = rsync_command(&cfg, "/remote/a b.txt", "/tmp/x", true, false) else { return }; // no ssh here
        let env: Vec<_> = cmd.get_envs().filter_map(|(k, v)| Some((k.to_string_lossy().into_owned(), v?.to_string_lossy().into_owned()))).collect();
        assert!(env.iter().any(|(k, v)| k == "RSYNC_OLD_ARGS" && v == "1"));
        assert!(env.iter().any(|(k, v)| k == "RSYNC_PROTECT_ARGS" && v == "0"));
        let args: Vec<String> = cmd.get_args().map(|a| a.to_string_lossy().into_owned()).collect();
        let e = &args[args.iter().position(|a| a == "-e").unwrap() + 1];
        assert!(e.contains("'-l' 'me@outlook.com'"), "{e}");
        assert!(args.contains(&"pc.local:'/remote/a b.txt'".to_string()), "{args:?}");
        assert!(!args.iter().any(|a| a.contains("@pc.local")));
    }

    /// Real rsync over ssh to this machine: ITDA_RSYNC_TEST=user@host (key login).
    #[test]
    #[ignore]
    fn rsync_over_ssh() {
        let Ok(spec) = std::env::var("ITDA_RSYNC_TEST") else { return };
        let (user, host) = spec.split_once('@').unwrap();
        let cfg = SshCfg { host: host.into(), port: 22, user: user.into(), pass: String::new(), vnc_port: 5900, key: String::new(), key_pass: String::new(), key_alias: String::new() };
        let root = std::env::temp_dir().join(format!("itda-rsync-{}", crate::store::new_id()));
        std::fs::create_dir_all(root.join("to")).unwrap();
        let from = root.join("it's a file.txt");
        std::fs::write(&from, b"over rsync").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&from, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        let to = root.join("to").join("got.txt");
        run_rsync(&cfg, from.to_str().unwrap(), to.to_str().unwrap(), true, false, &Control::default()).unwrap();
        assert_eq!(std::fs::read(&to).unwrap(), b"over rsync");
        // Without -p rsync gives a new file the source's bits (umask applied), no special bits.
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let m = std::fs::metadata(&to).unwrap().permissions().mode();
            assert!(m & 0o100 != 0 && m & 0o7000 == 0, "{m:o}");
        }
        std::fs::remove_dir_all(root).unwrap();
    }

    /// Executables stay executable: files and folders made by the copy get the source's
    /// rwx bits (never setuid/setgid/sticky); a folder merged into keeps its own mode.
    #[cfg(unix)]
    #[test]
    fn copy_keeps_permissions() {
        use std::os::unix::fs::PermissionsExt;
        let (r, mut a, mut b, mut cx) = fixture();
        let set = |p: &Path, m: u32| std::fs::set_permissions(p, std::fs::Permissions::from_mode(m)).unwrap();
        let mode = |p: &Path| std::fs::metadata(p).unwrap().permissions().mode() & 0o7777;
        let src = r.join("from/Tools");
        std::fs::create_dir(&src).unwrap();
        std::fs::write(src.join("run.sh"), b"#!/bin/sh\n").unwrap();
        set(&src.join("run.sh"), 0o755);
        std::fs::write(src.join("notes.txt"), b"x").unwrap();
        set(&src.join("notes.txt"), 0o640);
        std::fs::write(src.join("suid"), b"x").unwrap();
        set(&src.join("suid"), 0o4755);
        set(&src, 0o550); // read-only folder: its mode must come after its contents
        cx.copy(&mut a, &mut b, src.to_str().unwrap(), r.join("to/Tools").to_str().unwrap(), 0).unwrap();
        let dst = r.join("to/Tools");
        assert_eq!(mode(&dst.join("run.sh")), 0o755);
        assert_eq!(mode(&dst.join("notes.txt")), 0o640);
        assert_eq!(mode(&dst.join("suid")), 0o755, "no setuid");
        assert_eq!(mode(&dst), 0o550);
        // Merging into an existing folder leaves that folder's mode alone.
        let merged = r.join("to/Merged");
        std::fs::create_dir(&merged).unwrap();
        set(&merged, 0o700);
        set(&src, 0o755);
        cx.policy = "replace".into();
        cx.copy(&mut a, &mut b, src.to_str().unwrap(), merged.to_str().unwrap(), 0).unwrap();
        assert_eq!(mode(&merged), 0o700);
        assert_eq!(mode(&merged.join("run.sh")), 0o755);
        set(&dst, 0o755);
        std::fs::remove_dir_all(r).unwrap();
    }
}
