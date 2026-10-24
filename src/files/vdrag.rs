//! Remote files dragged out to the system as "virtual files" (Windows: FileGroupDescriptor
//! and FileContents): the drop target asks for the list when it needs it and then reads
//! each file as a stream, pulled over SFTP on demand. Nothing is staged on this PC and the
//! remote originals are only read.
use super::*;
use std::io::{Read, Write};
use std::sync::mpsc::{sync_channel, Receiver, SyncSender, TrySendError};

/// Most entries one drag may carry, as for any export.
const LIMIT: usize = 10_000;

#[derive(Clone, Debug, PartialEq)]
pub struct Item {
    /// Path inside the drop folder, components joined with '\'.
    pub rel: String,
    pub dir: bool,
    pub size: u64,
    pub modified: u64,
    path: String,
}

/// Opens SFTP sessions for the streams; one idle session is kept for the next file.
pub struct Remote {
    open: Box<dyn Fn(&Control) -> io::Result<Sftp> + Send + Sync>,
    idle: Mutex<Option<Sftp>>,
    control: Arc<Control>,
    /// The drag's own live entry, from its admission until the drop (or Explorer's copy of it)
    /// is over: the files it goes on to read belong to an accepted drag. 0 when over.
    batch: Mutex<u64>,
    /// Explorer copies on its own thread after the drop (IDataObjectAsyncCapability).
    operation: std::sync::atomic::AtomicBool,
}

impl Remote {
    /// The drag's source: the open file connection of `files[0].id`.
    pub fn of(files: &[export::File]) -> io::Result<Arc<Remote>> {
        let first = files.first().ok_or_else(|| error("파일을 선택하세요"))?;
        let cfg = connection(&first.id)?.cfg.clone();
        let remote = Self::new(Box::new(move |c| Sftp::connect(&cfg, c)));
        // Admitted and registered in one step: a quit that closed admissions counts it, or
        // it doesn't start.
        *remote.batch.lock().unwrap() = super::admit_stream(&remote.control).ok_or_else(|| error(super::CLOSING))?;
        Ok(remote)
    }

    pub(crate) fn new(open: Box<dyn Fn(&Control) -> io::Result<Sftp> + Send + Sync>) -> Arc<Remote> {
        Arc::new(Remote { open, idle: Mutex::new(None), control: Arc::new(Control::default()), batch: Mutex::new(0), operation: Default::default() })
    }

    /// Explorer started copying in the background (it ends with `end_operation`).
    #[cfg_attr(not(windows), allow(dead_code))]
    pub fn start_operation(&self) {
        self.operation.store(true, std::sync::atomic::Ordering::SeqCst);
    }
    #[cfg_attr(not(windows), allow(dead_code))]
    pub fn end_operation(&self) {
        self.operation.store(false, std::sync::atomic::Ordering::SeqCst);
        self.end_batch();
    }
    /// The drag-and-drop loop returned: the drag is over unless Explorer copies on.
    #[cfg_attr(not(windows), allow(dead_code))]
    pub fn drop_done(&self) {
        if !self.operation.load(std::sync::atomic::Ordering::SeqCst) { self.end_batch(); }
    }
    pub fn end_batch(&self) {
        let mut batch = self.batch.lock().unwrap();
        let id = std::mem::take(&mut *batch);
        if id != 0 { unregister(id); }
    }

    fn session(&self) -> io::Result<Sftp> {
        match self.idle.lock().unwrap().take() {
            Some(s) => Ok(s),
            None => (self.open)(&self.control),
        }
    }

    /// Everything the drop creates, folders before their contents. Refuses names this PC
    /// can't store (and symbolic links are left out) before anything is written.
    pub fn plan(&self, files: &[export::File]) -> io::Result<Vec<Item>> {
        let mut s = self.session()?;
        let mut out = Vec::new();
        for f in files {
            let path = f.path()?.to_string_lossy().replace('\\', "/");
            let e = s.stat(&path)?.ok_or_else(|| error(format!("파일이 없습니다: {}", f.name)))?;
            if e.link {
                continue;
            }
            walk(&mut s, &path, &f.name, &e, &mut out)?;
        }
        let mut unique = std::collections::HashSet::new();
        for item in &out {
            if !unique.insert(windows_name_key(&item.rel)) { return Err(error(format!("Windows에서 같은 이름이 되는 파일이 여러 개 있습니다: {}",item.rel))); }
        }
        *self.idle.lock().unwrap() = Some(s);
        Ok(out)
    }

    /// The bytes of `item`, downloading in the background while they're read.
    pub fn reader(self: &Arc<Self>, item: &Item) -> Reader {
        // A file of an accepted drag may still be read while Itda waits to quit; anything else
        // is a new stream and needs admission.
        // Decided and registered while the drag can't end meanwhile (end_batch takes this lock).
        let batch = self.batch.lock().unwrap();
        let live = if *batch != 0 { Some(register(&self.control)) } else { super::admit_stream(&self.control) };
        drop(batch);
        let Some(live) = live else { return Reader::refused() };
        let (tx, rx) = sync_channel::<io::Result<Vec<u8>>>(16);
        let (me, path, size) = (self.clone(), item.path.clone(), item.size);
        std::thread::spawn(move || {
            let mut s = match me.session() {
                Ok(s) => s,
                Err(e) => return drop(tx.try_send(Err(e))),
            };
            let mut sink = Chan { tx: &tx, control: &me.control };
            match s.download(&path, size, &mut sink, &me.control, |_| {}) {
                // Only a session that finished cleanly is reused.
                Ok(()) => *me.idle.lock().unwrap() = Some(s),
                // Never waits on a paused reader: with the channel full, the reader learns of
                // the failure when it finds the download gone short of the size.
                Err(e) => drop(tx.try_send(Err(e))),
            }
        });
        Reader::new(rx, size, self.control.clone(), live)
    }

    /// Stop every download still running for this drag.
    pub fn stop(&self) {
        self.control.stop();
    }
}

fn walk(s: &mut Sftp, path: &str, rel: &str, e: &Entry, out: &mut Vec<Item>) -> io::Result<()> {
    if out.len() >= LIMIT {
        return Err(error("한 번에 끌어 놓을 수 있는 항목 수(10,000개)를 넘었습니다"));
    }
    for part in rel.split('\\') {
        if name(part).is_err() || windows_target(part,rel).is_err() {
            return Err(error(format!("Windows에서 쓸 수 없는 파일 이름입니다: {}", rel.replace('\\', "/"))));
        }
    }
    // The receiver's own folder path is added in front; keep well under MAX_PATH.
    if destination_name(rel,true).encode_utf16().count() >= 200 {
        return Err(error(format!("경로가 너무 깁니다: {}", rel.replace('\\', "/"))));
    }
    out.push(Item { rel: destination_name(rel,true), dir: e.dir, size: if e.dir { 0 } else { e.size }, modified: e.modified, path: path.to_string() });
    if e.dir {
        let mut children = s.list(path)?;
        children.sort_by(|a, b| a.name.cmp(&b.name));
        for c in children.iter().filter(|c| c.name != "." && c.name != ".." && !c.link) {
            walk(s, &format!("{}/{}", path.trim_end_matches('/'), c.name), &format!("{rel}\\{}", c.name), c, out)?;
        }
    }
    Ok(())
}

/// The download's side of a stream. A paused reader (the channel full) doesn't hold up a
/// cancel: sending gives up as soon as the drag is stopped.
struct Chan<'a> {
    tx: &'a SyncSender<io::Result<Vec<u8>>>,
    control: &'a Control,
}
impl Write for Chan<'_> {
    fn write(&mut self, b: &[u8]) -> io::Result<usize> {
        let mut chunk = b.to_vec();
        loop {
            match self.tx.try_send(Ok(chunk)) {
                Ok(()) => return Ok(b.len()),
                // The reader is gone (the receiver cancelled): stop downloading.
                Err(TrySendError::Disconnected(_)) => return Err(io::Error::new(io::ErrorKind::BrokenPipe, "읽기를 멈췄습니다")),
                Err(TrySendError::Full(Ok(back))) => {
                    self.control.check()?;
                    chunk = back;
                    std::thread::sleep(std::time::Duration::from_millis(5));
                }
                Err(TrySendError::Full(Err(e))) => return Err(e),
            }
        }
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

/// Streams a drop target is reading right now, each with its drag's control: quitting asks
/// while any runs, and stopping them all stops their downloads.
static LIVE: Mutex<Vec<(u64, Arc<Control>)>> = Mutex::new(Vec::new());
static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
/// A new live entry (files::admit_stream checks admission first, under the job registry's lock).
pub(super) fn register(control: &Arc<Control>) -> u64 {
    let id = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    LIVE.lock().unwrap().push((id, control.clone()));
    id
}
fn unregister(id: u64) {
    LIVE.lock().unwrap().retain(|(l, _)| *l != id);
}
/// Streams still delivering (a stopped one has nothing of ours left to clean up).
#[cfg_attr(not(windows), allow(dead_code))]
pub fn active_streams() -> usize {
    LIVE.lock().unwrap().iter().filter(|(_, c)| c.check().is_ok()).count()
}
/// Stop every drag's downloads; their readers then report an error to the drop target.
#[cfg_attr(not(windows), allow(dead_code))]
pub fn stop_all() {
    for (_, c) in LIVE.lock().unwrap().iter() {
        c.stop();
    }
}

pub struct Reader {
    rx: Receiver<io::Result<Vec<u8>>>,
    buf: Vec<u8>,
    at: usize,
    /// What the drop target was told the file holds, and what it got so far.
    size: u64,
    delivered: u64,
    /// The live entry (0 once the stream ended: the whole size delivered, an error, or dropped).
    live: u64,
    control: Arc<Control>,
    /// Not admitted (Itda is quitting): every read says so.
    refused: bool,
}
impl Reader {
    fn new(rx: Receiver<io::Result<Vec<u8>>>, size: u64, control: Arc<Control>, live: u64) -> Reader {
        let mut r = Reader { rx, buf: Vec::new(), at: 0, size, delivered: 0, live, control, refused: false };
        if size == 0 { r.end(); }
        r
    }
    /// A stream that may not start (Itda is quitting): its first read fails.
    fn refused() -> Reader {
        let (_, rx) = sync_channel(0);
        Reader { rx, buf: Vec::new(), at: 0, size: 1, delivered: 0, live: 0, control: Arc::new(Control::default()), refused: true }
    }
    fn end(&mut self) {
        if self.live != 0 {
            unregister(std::mem::take(&mut self.live));
        }
    }
    fn fail(&mut self, e: io::Error) -> io::Result<usize> {
        self.end();
        Err(e)
    }
    /// These bytes, in chunks of `chunk` (tests).
    #[cfg(test)]
    pub(crate) fn of(bytes: &[u8], chunk: usize) -> Reader {
        let (tx, rx) = sync_channel(bytes.len() / chunk.max(1) + 2);
        for c in bytes.chunks(chunk.max(1)) {
            tx.send(Ok(c.to_vec())).unwrap();
        }
        let control = Arc::new(Control::default());
        let live = register(&control);
        Reader::new(rx, bytes.len() as u64, control, live)
    }
}
impl Drop for Reader {
    fn drop(&mut self) {
        self.end();
    }
}
impl Read for Reader {
    fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
        // Nothing asked for: nothing changes.
        if out.is_empty() {
            return Ok(0);
        }
        // Stopped: nothing more is handed out, buffered or not (the file isn't complete).
        if self.delivered < self.size && self.control.check().is_err() {
            return self.fail(io::Error::other("전송을 중단했습니다"));
        }
        while self.at == self.buf.len() {
            // The whole advertised size is out: end of file, without waiting for the download.
            if self.delivered >= self.size {
                self.end();
                return Ok(0);
            }
            if self.live == 0 {
                return Err(io::Error::other(if self.refused { super::CLOSING } else { "전송이 중단되었습니다" }));
            }
            match self.rx.recv() {
                Ok(Ok(b)) => (self.buf, self.at) = (b, 0),
                Ok(Err(e)) => return self.fail(e),
                // The download ended short of the size (stopped, or the connection went): an
                // error, never a shorter file passed off as complete.
                Err(_) => {
                    let why = if self.control.check().is_err() { "전송을 중단했습니다" } else { "원격 파일을 끝까지 읽지 못했습니다" };
                    return self.fail(io::Error::other(why));
                }
            }
        }
        let n = out.len().min(self.buf.len() - self.at);
        out[..n].copy_from_slice(&self.buf[self.at..self.at + n]);
        self.at += n;
        self.delivered += n as u64;
        if self.delivered >= self.size && self.at == self.buf.len() {
            self.end();
        }
        Ok(n)
    }
}

/// A local sftp-server as the remote (tests): ITDA_SFTP_SERVER, else the usual places.
#[cfg(test)]
pub(crate) fn test_server() -> Option<Arc<Remote>> {
    let server: PathBuf = std::env::var_os("ITDA_SFTP_SERVER")
        .map(PathBuf::from)
        .or_else(|| ["/usr/libexec/sftp-server", r"C:\Windows\System32\OpenSSH\sftp-server.exe"].into_iter().map(PathBuf::from).find(|p| p.is_file()))?;
    server.is_file().then(|| {
        Remote::new(Box::new(move |c| {
            let child = std::process::Command::new(&server)
                .stdin(std::process::Stdio::piped())
                .stdout(std::process::Stdio::piped())
                .stderr(std::process::Stdio::null())
                .spawn()?;
            Sftp::start(child, c)
        }))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plan_and_stream_a_tree() {
        let Some(remote) = test_server() else { return };
        let root = std::env::temp_dir().join(format!("itda-vdrag-{}", crate::store::new_id()));
        std::fs::create_dir_all(root.join("Folder/Sub")).unwrap();
        let big: Vec<u8> = (0..(3 << 20) + 17).map(|i| (i % 251) as u8).collect();
        std::fs::write(root.join("Folder/Sub/big.bin"), &big).unwrap();
        std::fs::write(root.join("Folder/한글.txt"), b"hello").unwrap();
        std::fs::write(root.join("one.txt"), b"one").unwrap();
        let file = |n: &str, dir: bool| export::File { id: String::new(), remote: true, source: root.to_string_lossy().into_owned(), name: n.into(), dir };
        let items = remote.plan(&[file("Folder", true), file("one.txt", false)]).unwrap();
        let rels: Vec<_> = items.iter().map(|i| (i.rel.as_str(), i.dir)).collect();
        assert_eq!(rels, [("Folder", true), ("Folder\\Sub", true), ("Folder\\Sub\\big.bin", false), ("Folder\\한글.txt", false), ("one.txt", false)]);
        let read = |rel: &str| {
            let mut got = Vec::new();
            remote.reader(items.iter().find(|i| i.rel == rel).unwrap()).read_to_end(&mut got).unwrap();
            got
        };
        assert_eq!(read("Folder\\Sub\\big.bin"), big);
        assert_eq!(read("Folder\\한글.txt"), b"hello");
        assert_eq!(read("one.txt"), b"one");
        // A reader dropped part-way stops its download and the next read still works.
        let mut r = remote.reader(&items[2]);
        let mut first = [0u8; 10];
        r.read_exact(&mut first).unwrap();
        drop(r);
        assert_eq!(read("one.txt"), b"one");
        // Names Windows can't store are refused while planning (on Windows such a file
        // can't exist: "bad:name" is a stream of "bad").
        #[cfg(not(windows))]
        {
            std::fs::write(root.join("Folder/bad:name"), b"x").unwrap();
            let e = remote.plan(&[file("Folder", true)]).unwrap_err();
            assert!(e.to_string().contains("Folder/bad:name"), "{e}");
        }
        std::fs::remove_dir_all(root).unwrap();
    }

    fn live(r: &Reader) -> bool {
        LIVE.lock().unwrap().iter().any(|(l, _)| *l == r.live && r.live != 0)
    }

    /// A stream counts as running until its last advertised byte is read (not until the drop
    /// target lets go of it), and an empty read changes nothing.
    #[test]
    fn a_stream_ends_at_its_last_byte() {
        let mut r = Reader::of(b"hello world", 4);
        assert!(live(&r));
        assert_eq!(r.read(&mut []).unwrap(), 0);
        assert!(live(&r), "an empty read ended the stream");
        let mut got = [0u8; 11];
        r.read_exact(&mut got).unwrap();
        assert_eq!(&got, b"hello world");
        assert!(!live(&r), "still running after its last byte");
        assert_eq!(r.read(&mut [0u8; 4]).unwrap(), 0, "the extra read at the end");
        // Dropped early: no longer running.
        let r = Reader::of(b"abcdef", 2);
        let id = r.live;
        drop(r);
        assert!(!LIVE.lock().unwrap().iter().any(|(l, _)| *l == id));
        // An empty file is done from the start.
        assert!(!live(&Reader::of(b"", 1)));
    }

    /// A download that stops short is an error to the drop target, never a shorter file.
    #[test]
    fn a_stream_cut_short_is_an_error() {
        let (tx, rx) = sync_channel(4);
        tx.send(Ok(b"abc".to_vec())).unwrap();
        drop(tx);
        let c = Arc::new(Control::default());
        let mut r = Reader::new(rx, 10, c.clone(), register(&c));
        let mut got = [0u8; 3];
        r.read_exact(&mut got).unwrap();
        assert!(r.read(&mut [0u8; 4]).is_err(), "a short download looked complete");
        assert!(!live(&r));
        let (tx, rx) = sync_channel(4);
        tx.send(Err(io::Error::other("network"))).unwrap();
        let c = Arc::new(Control::default());
        let mut r = Reader::new(rx, 10, c.clone(), register(&c));
        assert!(r.read(&mut [0u8; 4]).is_err() && !live(&r));
    }

    /// Stopping a drag ends a download that waits on a paused reader (the channel full), and
    /// the reader then gets an error.
    #[test]
    fn stopping_unblocks_a_paused_stream() {
        let (tx, rx) = sync_channel::<io::Result<Vec<u8>>>(1);
        let control = Arc::new(Control::default());
        let mut r = Reader::new(rx, 1 << 20, control.clone(), register(&control));
        let c = control.clone();
        let producer = std::thread::spawn(move || {
            let mut sink = Chan { tx: &tx, control: &c };
            loop { if let Err(e) = sink.write(&[7u8; 1024]) { return e; } }
        });
        std::thread::sleep(std::time::Duration::from_millis(50));
        assert_eq!(active_streams_of(&control), 1);
        // (What stop_all does for every live stream; all of them would stop other tests' too.)
        control.stop();
        let e = producer.join().unwrap();
        assert_ne!(e.kind(), io::ErrorKind::BrokenPipe, "the producer only ended because the reader went");
        assert_eq!(active_streams_of(&control), 0, "a stopped stream still counts as running");
        let mut buf = vec![0u8; 4096];
        let mut failed = false;
        for _ in 0..4 { if r.read(&mut buf).is_err() { failed = true; break; } }
        assert!(failed, "the reader of a stopped stream saw no error");
    }
    fn active_streams_of(c: &Arc<Control>) -> usize {
        LIVE.lock().unwrap().iter().filter(|(_, x)| Arc::ptr_eq(x, c) && x.check().is_ok()).count()
    }

    /// While Itda waits to quit, an accepted drag still gets its later files; once the drag is
    /// over, a new stream is refused (it would start after the count).
    #[test]
    fn an_accepted_drag_reads_on_while_closing() {
        let remote = Remote::new(Box::new(|_| Err(io::Error::other("no server in this test"))));
        *remote.batch.lock().unwrap() = super::super::admit_stream(&remote.control).expect("admitted");
        super::super::close_admissions();
        let item = Item { rel: "a".into(), dir: false, size: 3, modified: 0, path: "/nowhere/a".into() };
        let r = remote.reader(&item);
        assert!(r.live != 0 && !r.refused, "a file of an accepted drag was refused");
        drop(r);
        remote.end_batch();
        let mut r = remote.reader(&item);
        assert!(r.refused);
        let e = r.read(&mut [0u8; 4]).unwrap_err();
        assert_eq!(e.to_string(), super::super::CLOSING);
        assert!(super::super::admit_stream(&remote.control).is_none());
        super::super::open_admissions();
    }

    /// A stopped stream hands out nothing more, even bytes already buffered.
    #[test]
    fn a_stopped_stream_delivers_no_buffered_bytes() {
        let (tx, rx) = sync_channel(4);
        tx.send(Ok(b"all of it".to_vec())).unwrap();
        drop(tx);
        let c = Arc::new(Control::default());
        let mut r = Reader::new(rx, 9, c.clone(), register(&c));
        assert_eq!(r.read(&mut []).unwrap(), 0);
        c.stop();
        let mut buf = [0u8; 16];
        assert!(r.read(&mut buf).is_err(), "a stopped stream delivered its buffer");
        assert_eq!(buf, [0u8; 16]);
        assert!(!live(&r));
        // A finished one keeps saying end of file.
        let mut done = Reader::of(b"ok", 2);
        let mut two = [0u8; 2];
        done.read_exact(&mut two).unwrap();
        done.control.stop();
        assert_eq!(done.read(&mut two).unwrap(), 0);
    }
}
