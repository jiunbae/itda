//! Remote files dragged out to the system as "virtual files" (Windows: FileGroupDescriptor
//! and FileContents): the drop target asks for the list when it needs it and then reads
//! each file as a stream, pulled over SFTP on demand. Nothing is staged on this PC and the
//! remote originals are only read.
use super::*;
use std::io::{Read, Write};
use std::sync::mpsc::{sync_channel, Receiver};

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
}

impl Remote {
    /// The drag's source: the open file connection of `files[0].id`.
    pub fn of(files: &[export::File]) -> io::Result<Arc<Remote>> {
        let first = files.first().ok_or_else(|| error("파일을 선택하세요"))?;
        let cfg = connection(&first.id)?.cfg.clone();
        Ok(Self::new(Box::new(move |c| Sftp::connect(&cfg, c))))
    }

    pub(crate) fn new(open: Box<dyn Fn(&Control) -> io::Result<Sftp> + Send + Sync>) -> Arc<Remote> {
        Arc::new(Remote { open, idle: Mutex::new(None), control: Arc::new(Control::default()) })
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
        let (tx, rx) = sync_channel::<io::Result<Vec<u8>>>(16);
        let (me, path, size) = (self.clone(), item.path.clone(), item.size);
        std::thread::spawn(move || {
            let mut s = match me.session() {
                Ok(s) => s,
                Err(e) => return drop(tx.send(Err(e))),
            };
            let mut sink = Chan(&tx);
            match s.download(&path, size, &mut sink, &me.control, |_| {}) {
                // Only a session that finished cleanly is reused.
                Ok(()) => *me.idle.lock().unwrap() = Some(s),
                Err(e) => drop(tx.send(Err(e))),
            }
        });
        { STREAMS.fetch_add(1, std::sync::atomic::Ordering::Relaxed); Reader { rx, buf: Vec::new(), at: 0, done: false } }
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

struct Chan<'a>(&'a std::sync::mpsc::SyncSender<io::Result<Vec<u8>>>);
impl Write for Chan<'_> {
    fn write(&mut self, b: &[u8]) -> io::Result<usize> {
        // The reader is gone (the receiver cancelled): stop downloading.
        self.0.send(Ok(b.to_vec())).map_err(|_| io::Error::new(io::ErrorKind::BrokenPipe, "읽기를 멈췄습니다"))?;
        Ok(b.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

/// Files being read by a drop target right now (quitting would cut them off).
static STREAMS: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
#[cfg_attr(not(windows), allow(dead_code))]
pub fn active_streams() -> usize {
    STREAMS.load(std::sync::atomic::Ordering::Relaxed)
}

pub struct Reader {
    rx: Receiver<io::Result<Vec<u8>>>,
    buf: Vec<u8>,
    at: usize,
    done: bool,
}
impl Reader {
    /// These bytes, in chunks of `chunk` (tests).
    #[cfg(test)]
    pub(crate) fn of(bytes: &[u8], chunk: usize) -> Reader {
        let (tx, rx) = sync_channel(bytes.len() / chunk.max(1) + 2);
        for c in bytes.chunks(chunk.max(1)) {
            tx.send(Ok(c.to_vec())).unwrap();
        }
        { STREAMS.fetch_add(1, std::sync::atomic::Ordering::Relaxed); Reader { rx, buf: Vec::new(), at: 0, done: false } }
    }
}
impl Drop for Reader {
    fn drop(&mut self) {
        STREAMS.fetch_sub(1, std::sync::atomic::Ordering::Relaxed);
    }
}
impl Read for Reader {
    fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
        while self.at == self.buf.len() {
            if self.done {
                return Ok(0);
            }
            match self.rx.recv() {
                Ok(Ok(b)) => (self.buf, self.at) = (b, 0),
                Ok(Err(e)) => {
                    self.done = true;
                    return Err(e);
                }
                Err(_) => self.done = true, // sender finished: end of file
            }
        }
        let n = out.len().min(self.buf.len() - self.at);
        out[..n].copy_from_slice(&self.buf[self.at..self.at + n]);
        self.at += n;
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
}
