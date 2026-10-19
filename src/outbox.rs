//! Everything a server sends one viewer, written by a thread of its own so a viewer (or link)
//! that stops reading doesn't stop the server: the RDP bridge keeps reading the host and
//! forwarding input, and the screen share keeps answering. Messages go out whole and in the
//! order they were queued; the pump asks `room()` before encoding the next update, so the
//! queue holds about LIMIT bytes plus at most one update past it.

use std::io::{self, Write};
use std::net::{Shutdown, TcpStream};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering::SeqCst};
use std::sync::mpsc::{channel, sync_channel, Receiver, Sender, SyncSender};
use std::sync::Arc;
use std::time::Duration;

/// Queued bytes past which the pump waits before encoding another update.
const LIMIT: usize = 512 << 10;
/// Past these the viewer isn't reading at all: give up on it. (Also a ceiling on one update:
/// a bigger one, say a noisy 8K screen in Raw, ends the connection.)
const HARD_BYTES: usize = 64 << 20;
const HARD_MSGS: usize = 1024;
/// One blocked write this long ends the connection.
const WRITE_TIMEOUT: Duration = Duration::from_secs(30);
/// Written buffers kept for reuse, and the largest one worth keeping.
const POOL: usize = 4;
const POOL_CAP: usize = 4 << 20;

struct Shared {
    /// Bytes and messages queued or being written.
    bytes: AtomicUsize,
    msgs: AtomicUsize,
    dead: AtomicBool,
    sock: TcpStream,
}

impl Shared {
    fn kill(&self) {
        self.dead.store(true, SeqCst);
        let _ = self.sock.shutdown(Shutdown::Both); // may already be shut: fine
    }
}

pub struct Outbox {
    tx: Option<Sender<Vec<u8>>>,
    pool: Receiver<Vec<u8>>,
    shared: Arc<Shared>,
}

impl Outbox {
    /// Take over writing to `sock` (the viewer's socket; reads stay with the caller).
    pub fn new(sock: TcpStream) -> io::Result<Outbox> {
        sock.set_write_timeout(Some(WRITE_TIMEOUT))?;
        let shared = Arc::new(Shared { bytes: AtomicUsize::new(0), msgs: AtomicUsize::new(0), dead: AtomicBool::new(false), sock: sock.try_clone()? });
        let (tx, rx) = channel::<Vec<u8>>();
        let (pool_tx, pool) = sync_channel(POOL);
        let s = shared.clone();
        std::thread::spawn(move || write_loop(sock, rx, pool_tx, &s));
        Ok(Outbox { tx: Some(tx), pool, shared })
    }

    /// Is there room for another framebuffer update?
    pub fn room(&self) -> bool {
        self.shared.bytes.load(SeqCst) < LIMIT
    }

    /// Bytes queued or being written.
    #[cfg(test)]
    pub fn queued(&self) -> usize {
        self.shared.bytes.load(SeqCst)
    }

    /// The connection is gone (write failed, timed out, or the viewer stopped reading).
    pub fn dead(&self) -> bool {
        self.shared.dead.load(SeqCst)
    }

    /// An empty buffer to build the next message in (a written one when available).
    pub fn buffer(&self) -> Vec<u8> {
        self.pool.try_recv().unwrap_or_default()
    }

    /// Queue a whole message. Never blocks; an error means the connection is over.
    pub fn send(&self, b: Vec<u8>) -> io::Result<()> {
        if b.is_empty() {
            return Ok(());
        }
        let s = &self.shared;
        let n = b.len();
        let bytes = s.bytes.fetch_add(n, SeqCst) + n;
        let msgs = s.msgs.fetch_add(1, SeqCst) + 1;
        let fail = |why: &str| {
            s.bytes.fetch_sub(n, SeqCst);
            s.msgs.fetch_sub(1, SeqCst);
            s.kill();
            Err(io::Error::new(io::ErrorKind::BrokenPipe, why.to_string()))
        };
        if s.dead.load(SeqCst) {
            return fail("viewer connection closed");
        }
        if bytes > HARD_BYTES || msgs > HARD_MSGS {
            return fail("viewer stopped reading");
        }
        match &self.tx {
            Some(tx) if tx.send(b).is_ok() => Ok(()),
            _ => fail("viewer connection closed"),
        }
    }
}

impl Drop for Outbox {
    /// The pump is done: end the connection now rather than drain a queue nobody waits for.
    fn drop(&mut self) {
        self.tx.take();
        self.shared.kill();
    }
}

fn write_loop(mut sock: TcpStream, rx: Receiver<Vec<u8>>, pool: SyncSender<Vec<u8>>, s: &Shared) {
    while let Ok(mut b) = rx.recv() {
        if s.dead.load(SeqCst) || sock.write_all(&b).is_err() {
            s.kill();
            return;
        }
        s.bytes.fetch_sub(b.len(), SeqCst);
        s.msgs.fetch_sub(1, SeqCst);
        if b.capacity() <= POOL_CAP {
            b.clear();
            let _ = pool.try_send(b); // pool full: drop it
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Read;
    use std::net::TcpListener;
    use std::time::Instant;

    fn pair() -> (TcpStream, TcpStream) {
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        let a = TcpStream::connect(l.local_addr().unwrap()).unwrap();
        let (b, _) = l.accept().unwrap();
        (a, b)
    }

    /// Wait (2 s at most) for `ok`.
    fn until(what: &str, ok: impl Fn() -> bool) {
        let deadline = Instant::now() + Duration::from_secs(2);
        while !ok() {
            assert!(Instant::now() < deadline, "{what}");
            std::thread::sleep(Duration::from_millis(2));
        }
    }

    /// A viewer that doesn't read: sending never blocks, room() closes, and once it reads
    /// everything arrives whole and in order.
    #[test]
    fn slow_viewer() {
        let (srv, mut viewer) = pair();
        let out = Outbox::new(srv).unwrap();
        let t = Instant::now();
        let mut sent = Vec::new();
        let mut i = 0u32;
        while out.room() {
            let mut b = out.buffer();
            b.extend(std::iter::repeat_n((i % 251) as u8, 64 << 10));
            b[..4].copy_from_slice(&i.to_be_bytes());
            sent.push(b.clone());
            out.send(b).unwrap();
            i += 1;
            assert!(i < 10_000, "room() never closed");
        }
        out.send(vec![9; 10]).unwrap(); // control messages still queue
        assert!(t.elapsed() < Duration::from_secs(2), "send must not block");
        assert!(!out.dead());
        for want in &sent {
            let mut got = vec![0u8; want.len()];
            viewer.read_exact(&mut got).unwrap();
            assert!(&got == want, "message {} intact and in order", u32::from_be_bytes(got[..4].try_into().unwrap()));
        }
        let mut tail = [0u8; 10];
        viewer.read_exact(&mut tail).unwrap();
        assert_eq!(tail, [9; 10]);
        until("everything written", || out.shared.msgs.load(SeqCst) == 0);
        assert!(out.room() && out.queued() == 0);
    }

    /// A viewer that never reads is given up on at the hard limit, and a read blocked on the
    /// same socket returns: at once on macOS and Linux; on Windows, where shutdown doesn't
    /// interrupt a blocked recv, once the viewer's side closes on the FIN it got.
    #[test]
    fn stuck_viewer() {
        let (srv, viewer) = pair();
        let reader = srv.try_clone().unwrap();
        let out = Outbox::new(srv).unwrap();
        let (woke, wait) = channel();
        std::thread::spawn(move || woke.send((&reader).read(&mut [0u8; 1]).map_or(0, |n| n + 1)));
        let mut n = 0;
        let err = loop {
            match out.send(vec![1; 1 << 20]) {
                Ok(()) => n += 1,
                Err(e) => break e,
            }
            assert!(n < 1000);
        };
        assert_eq!(err.kind(), io::ErrorKind::BrokenPipe);
        assert!(out.dead());
        assert!(out.send(vec![1]).is_err());
        let mut woke = wait.recv_timeout(Duration::from_secs(2));
        if cfg!(windows) && woke.is_err() {
            drop(viewer);
            woke = wait.recv_timeout(Duration::from_secs(2));
        }
        assert!(matches!(woke, Ok(0 | 1)), "reader woke up (EOF or error)");
    }

    /// Dropping the outbox ends the connection: the viewer sees EOF.
    #[test]
    fn drop_closes() {
        let (srv, mut viewer) = pair();
        let out = Outbox::new(srv).unwrap();
        out.send(b"hi".to_vec()).unwrap();
        let mut hi = [0u8; 2];
        viewer.read_exact(&mut hi).unwrap();
        assert_eq!(&hi, b"hi");
        drop(out);
        viewer.set_read_timeout(Some(Duration::from_secs(2))).unwrap();
        assert_eq!(viewer.read(&mut [0u8; 1]).unwrap(), 0, "EOF");
    }

    /// The pool keeps written buffers, never huge ones.
    #[test]
    fn recycles_buffers() {
        let (srv, mut viewer) = pair();
        let out = Outbox::new(srv).unwrap();
        let mut b = out.buffer();
        b.reserve(1000);
        b.push(1);
        out.send(b).unwrap();
        out.send(vec![2; POOL_CAP + 1]).unwrap();
        let mut got = vec![0u8; 1 + POOL_CAP + 1];
        viewer.read_exact(&mut got).unwrap();
        let reused = out.pool.recv_timeout(Duration::from_secs(2)).expect("the small buffer comes back");
        assert!(reused.is_empty() && reused.capacity() >= 1000);
        until("everything written", || out.shared.msgs.load(SeqCst) == 0);
        assert!(out.pool.try_recv().is_err(), "the oversized buffer was dropped");
    }
}
