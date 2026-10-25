//! SFTP v3 over the system OpenSSH transport. No remote shell quoting or text listings.
use super::{Control, Entry};
use crate::tunnel::SshCfg;
use std::io::{self, Read, Write};
use std::process::{Child, ChildStdin, ChildStdout, Stdio};
use std::sync::{Arc, Mutex};
const MAX_PACKET: usize = 4 << 20;
const CHUNK: usize = 32 << 10;
const WINDOW: usize = 64;
fn err(s: impl Into<String>) -> io::Error {
    io::Error::other(s.into())
}
fn string(out: &mut Vec<u8>, s: &[u8]) {
    out.extend_from_slice(&(s.len() as u32).to_be_bytes());
    out.extend_from_slice(s);
}
fn path(out: &mut Vec<u8>, s: &str) {
    string(out, s.as_bytes());
}
struct Packet {
    t: u8,
    id: u32,
    b: Vec<u8>,
    at: usize,
}
impl Packet {
    fn u32(&mut self) -> io::Result<u32> {
        Ok(u32::from_be_bytes(self.take(4)?.try_into().unwrap()))
    }
    fn u64(&mut self) -> io::Result<u64> {
        Ok(u64::from_be_bytes(self.take(8)?.try_into().unwrap()))
    }
    fn take(&mut self, n: usize) -> io::Result<&[u8]> {
        let end = self
            .at
            .checked_add(n)
            .filter(|&e| e <= self.b.len())
            .ok_or_else(|| err("잘못된 SFTP 응답"))?;
        let start = self.at;
        self.at = end;
        Ok(&self.b[start..end])
    }
    fn bytes(&mut self) -> io::Result<Vec<u8>> {
        let n = self.u32()? as usize;
        Ok(self.take(n)?.to_vec())
    }
    fn text(&mut self) -> io::Result<String> {
        String::from_utf8(self.bytes()?).map_err(|_| err("UTF-8로 표시할 수 없는 파일 이름입니다"))
    }
    fn status(&mut self) -> io::Result<u32> {
        if self.t != 101 {
            return Err(err("SFTP 상태 응답이 아닙니다"));
        }
        let code = self.u32()?;
        if code == 0 || code == 1 || code == 2 {
            return Ok(code);
        }
        let text = self.text().unwrap_or_default();
        // The server's own words stay for diagnosis; the common refusals get a reason.
        let reason = match code {
            3 => "원격 기기에서 권한이 없습니다",
            4 => "원격 기기가 작업을 거부했습니다. 같은 이름이 있거나 쓸 수 없는 이름인지 확인하세요",
            5 => "원격 기기가 이 경로나 이름을 받아들이지 않습니다",
            _ => return Err(err(format!("SFTP: {text} (코드 {code})"))),
        };
        Err(err(format!("{reason} (SFTP 코드 {code}: {text})")))
    }
    fn attrs(&mut self, name: String) -> io::Result<Entry> {
        let f = self.u32()?;
        let size = if f & 1 != 0 { self.u64()? } else { 0 };
        if f & 2 != 0 {
            self.take(8)?;
        }
        let mode = if f & 4 != 0 { self.u32()? } else { 0 };
        let modified = if f & 8 != 0 {
            self.u32()?;
            self.u32()? as u64
        } else {
            0
        };
        if f & 0x80000000 != 0 {
            let n = self.u32()?;
            for _ in 0..n {
                self.bytes()?;
                self.bytes()?;
            }
        }
        Ok(Entry {
            name,
            size,
            modified,
            dir: mode & 0xf000 == 0x4000,
            link: mode & 0xf000 == 0xa000,
            mode: (f & 4 != 0).then_some(mode & 0o777),
        })
    }
}
pub struct Sftp {
    child: Arc<Mutex<Child>>,
    input: ChildStdin,
    output: ChildStdout,
    next: u32,
    diagnostics: Arc<Mutex<String>>,
    /// Requests whose reply is read and dropped when it arrives (a folder's CLOSE): nothing
    /// waits a round trip for it.
    deferred: std::collections::VecDeque<u32>,
}
// At most WINDOW writes are outstanding. Source reads are already bounded to WINDOW
// CHUNKs by download(); bytes flow directly to the other SFTP session, never a local file.
struct RelayWriter<'a, F: FnMut(u64)> {
    dst: &'a mut Sftp,
    handle: Vec<u8>,
    pending: Vec<u32>,
    offset: u64,
    control: &'a Control,
    progress: F,
}
impl<F: FnMut(u64)> Write for RelayWriter<'_, F> {
    fn write(&mut self, data: &[u8]) -> io::Result<usize> {
        for data in data.chunks(CHUNK) {
            self.control.check()?;
            let mut b = Vec::new();
            string(&mut b, &self.handle);
            b.extend_from_slice(&self.offset.to_be_bytes());
            string(&mut b, data);
            self.pending.push(self.dst.send(6, &b)?);
            self.offset += data.len() as u64;
            if self.pending.len() >= WINDOW { self.flush()?; }
        }
        Ok(data.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        let mut failure = None;
        while !self.pending.is_empty() {
            self.control.check()?;
            let mut r = self.dst.packet()?;
            let i = self.pending.iter().position(|&id| id == r.id).ok_or_else(|| err("SFTP 중계 쓰기 응답 오류"))?;
            self.pending.remove(i);
            let checked = r.status().and_then(|status| if status == 0 { Ok(()) } else { Err(err("SFTP 중계 쓰기 실패")) });
            // Drain the rest even after a write rejection: CLOSE and staging cleanup
            // must not mistake an old WRITE reply for their own response.
            if let Err(e) = checked { if failure.is_none() { failure = Some(e); } }
        }
        if let Some(e) = failure { return Err(e); }
        (self.progress)(self.offset);
        Ok(())
    }
}
/// A READDIR reply's entries. Every field is read; an entry whose name isn't UTF-8 or that
/// name() refuses (macOS "Icon\r", ':' for a Windows client) is left out instead of failing
/// the whole folder.
fn names(r: &mut Packet, out: &mut Vec<Entry>) -> io::Result<()> {
    let n = r.u32()?;
    for _ in 0..n {
        let name = String::from_utf8(r.bytes()?).ok();
        r.bytes()?;
        let entry = r.attrs(name.clone().unwrap_or_default())?;
        if name.is_some_and(|n| n != "." && n != ".." && super::name(&n).is_ok()) {
            out.push(entry);
        }
    }
    Ok(())
}
impl Drop for Sftp {
    fn drop(&mut self) {
        if let Ok(mut c) = self.child.lock() {
            let _ = c.kill();
            let _ = c.wait();
        }
    }
}
impl Sftp {
    pub fn connect(cfg: &SshCfg, control: &Control) -> io::Result<Self> {
        let mut cmd = crate::tunnel::ssh_command(cfg)?;
        cmd.args(["-o", "ControlMaster=no", "-o", "ControlPath=none"]);
        cmd.arg("-s")
            .arg(crate::tunnel::ssh_target(cfg)?)
            .arg("sftp");
        let child = cmd
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()?;
        Self::start(child, control)
    }
    pub(super) fn start(mut child: Child, control: &Control) -> io::Result<Self> {
        crate::tunnel::register_child(&child);
        let input = child.stdin.take().ok_or_else(|| err("SSH 입력 없음"))?;
        let output = child.stdout.take().ok_or_else(|| err("SSH 출력 없음"))?;
        let diagnostics = Arc::new(Mutex::new(String::new()));
        if let Some(pipe) = child.stderr.take() {
            let log = diagnostics.clone();
            std::thread::spawn(move || {
                use std::io::BufRead;
                for line in std::io::BufReader::new(pipe).lines().map_while(Result::ok) {
                    let mut log = log.lock().unwrap();
                    log.push_str(&line);
                    log.push('\n');
                    if log.len() > 8192 {
                        let at = log
                            .char_indices()
                            .find(|(i, _)| *i >= 4096)
                            .map_or(0, |(i, _)| i);
                        log.drain(..at);
                    }
                }
            });
        }
        let child = Arc::new(Mutex::new(child));
        control.attach(child.clone())?;
        let mut s = Self {
            child,
            input,
            output,
            next: 0,
            diagnostics,
            deferred: Default::default(),
        };
        s.write_packet(&[1, 0, 0, 0, 3])?;
        let p = s.packet()?;
        if p.t != 2 || p.id != 3 {
            return Err(err("SFTP v3를 지원하지 않는 서버입니다"));
        }
        Ok(s)
    }
    fn write_packet(&mut self, b: &[u8]) -> io::Result<()> {
        // One write per packet (the pipe isn't buffered).
        let mut out = Vec::with_capacity(4 + b.len());
        out.extend_from_slice(&(b.len() as u32).to_be_bytes());
        out.extend_from_slice(b);
        self.input.write_all(&out)?;
        self.input.flush()
    }
    fn packet(&mut self) -> io::Result<Packet> {
        loop {
            let p = self.raw_packet()?;
            // A reply nobody waits for is dropped whenever it arrives (servers may answer
            // in any order).
            if let Some(i) = self.deferred.iter().position(|id| *id == p.id) {
                self.deferred.remove(i);
                continue;
            }
            return Ok(p);
        }
    }
    fn raw_packet(&mut self) -> io::Result<Packet> {
        let mut len = [0; 4];
        self.output.read_exact(&mut len).map_err(|_| {
            let log = self.diagnostics.lock().unwrap();
            if log.is_empty() {
                err("파일 연결이 끊겼습니다. SSH 로그인과 원격 로그인 설정을 확인하세요")
            } else {
                crate::tunnel::classify(&log)
            }
        })?;
        let n = u32::from_be_bytes(len) as usize;
        if !(5..=MAX_PACKET).contains(&n) {
            return Err(err("SFTP 응답 크기가 잘못되었습니다"));
        }
        let mut b = vec![0; n];
        self.output.read_exact(&mut b)?;
        Ok(Packet {
            t: b[0],
            id: u32::from_be_bytes(b[1..5].try_into().unwrap()),
            b: b[5..].to_vec(),
            at: 0,
        })
    }
    fn send(&mut self, t: u8, b: &[u8]) -> io::Result<u32> {
        self.next = self.next.wrapping_add(1);
        let mut out = vec![t];
        out.extend_from_slice(&self.next.to_be_bytes());
        out.extend_from_slice(b);
        self.write_packet(&out)?;
        Ok(self.next)
    }
    fn request(&mut self, t: u8, b: &[u8]) -> io::Result<Packet> {
        let id = self.send(t, b)?;
        let p = self.packet()?;
        if p.id != id {
            return Err(err("SFTP 요청 순서 불일치"));
        }
        Ok(p)
    }
    fn status_request(&mut self, t: u8, b: &[u8]) -> io::Result<()> {
        let mut p = self.request(t, b)?;
        if p.status()? != 0 {
            return Err(err("파일 또는 폴더가 없습니다"));
        }
        Ok(())
    }
    fn handle(&mut self, t: u8, b: &[u8]) -> io::Result<Vec<u8>> {
        let mut p = self.request(t, b)?;
        if p.t == 101 {
            p.status()?;
            return Err(err("파일을 열 수 없습니다"));
        }
        if p.t != 102 {
            return Err(err("SFTP 핸들 응답 오류"));
        }
        p.bytes()
    }
    fn close(&mut self, h: &[u8]) -> io::Result<()> {
        let mut b = Vec::new();
        string(&mut b, h);
        self.status_request(4, &b)
    }
    /// CLOSE a handle without waiting for its reply (a read-only folder handle: there's
    /// nothing to confirm).
    fn close_later(&mut self, h: &[u8]) -> io::Result<()> {
        let mut b = Vec::new();
        string(&mut b, h);
        let id = self.send(4, &b)?;
        self.deferred.push_back(id);
        Ok(())
    }
    pub fn realpath(&mut self, p: &str) -> io::Result<String> {
        let mut b = Vec::new();
        path(&mut b, p);
        let mut r = self.request(16, &b)?;
        if r.t == 101 {
            r.status()?;
            return Err(err("폴더를 찾을 수 없습니다"));
        }
        if r.t != 104 || r.u32()? != 1 {
            return Err(err("SFTP 경로 응답 오류"));
        }
        r.text()
    }
    pub fn stat(&mut self, p: &str) -> io::Result<Option<Entry>> {
        let mut b = Vec::new();
        path(&mut b, p);
        let mut r = self.request(7, &b)?;
        if r.t == 101 {
            let code = r.status()?;
            return if code == 2 {
                Ok(None)
            } else {
                Err(err("파일 정보를 읽을 수 없습니다"))
            };
        }
        if r.t != 105 {
            return Err(err("SFTP 속성 응답 오류"));
        }
        Ok(Some(r.attrs(p.rsplit('/').next().unwrap_or(p).into())?))
    }
    pub fn list(&mut self, p: &str) -> io::Result<Vec<Entry>> {
        let mut b = Vec::new();
        path(&mut b, p);
        let h = self.handle(11, &b)?;
        let mut out = Vec::new();
        let mut b = Vec::new();
        string(&mut b, &h);
        // A few READDIRs ahead, so a large folder costs fewer round trips. Replies may come in
        // any order: each is matched to its request, entries from every NAME reply count (an
        // "end" for one request doesn't end those already answered), and no new READDIR goes
        // out after an "end".
        const AHEAD: usize = 3;
        let mut sent = std::collections::VecDeque::new();
        for _ in 0..AHEAD {
            sent.push_back(self.send(12, &b)?);
        }
        let mut done = false;
        let outcome = loop {
            if sent.is_empty() {
                break Ok(());
            }
            let mut r = self.packet()?;
            let Some(i) = sent.iter().position(|id| *id == r.id) else {
                return Err(err("SFTP 요청 순서 불일치"));
            };
            sent.remove(i);
            if r.t == 101 {
                if r.status().is_ok_and(|s| s == 1) {
                    done = true;
                    continue;
                }
                break Err(err("폴더 목록을 읽을 수 없습니다"));
            }
            if r.t != 104 {
                break Err(err("SFTP 목록 응답 오류"));
            }
            if !done {
                sent.push_back(self.send(12, &b)?);
            }
            // (A malformed reply is refused, but the stream stays in step: it was read whole.)
            if let Err(e) = names(&mut r, &mut out) {
                break Err(e);
            }
            if out.len() > 200_000 {
                break Err(err("폴더가 너무 큽니다. 하위 폴더로 이동하세요"));
            }
        };
        // Replies still due on a refused folder are dropped as they arrive, like the CLOSE.
        self.deferred.extend(sent);
        self.close_later(&h)?;
        outcome.map(|()| out)
    }
    pub fn mkdir(&mut self, p: &str) -> io::Result<()> {
        let mut b = Vec::new();
        path(&mut b, p);
        b.extend_from_slice(&0u32.to_be_bytes());
        self.status_request(14, &b)
    }
    pub fn rename(&mut self, from: &str, to: &str, replace: bool) -> io::Result<()> {
        let result = self.rename_once(from, to, replace);
        if result.is_ok() || replace || from == to || from.to_lowercase() != to.to_lowercase() {
            return result;
        }
        // A case-only change: a case-insensitive remote (Windows, macOS) sees the new name as
        // taken. Go through a unique temporary name, putting the item back if that fails.
        let at = from.rfind(['/', '\\']).map_or(0, |i| i + 1);
        let temp = format!("{}.itda-{}.rename", &from[..at], crate::store::new_id());
        if self.rename_once(from, &temp, false).is_err() {
            return result;
        }
        if let Err(e) = self.rename_once(&temp, to, false) {
            let _ = self.rename_once(&temp, from, false);
            return Err(e);
        }
        Ok(())
    }
    fn rename_once(&mut self, from: &str, to: &str, replace: bool) -> io::Result<()> {
        let mut b = Vec::new();
        if replace {
            path(&mut b, "posix-rename@openssh.com");
        }
        path(&mut b, from);
        path(&mut b, to);
        self.status_request(if replace { 200 } else { 18 }, &b)
    }
    pub fn remove(&mut self, p: &str, dir: bool) -> io::Result<()> {
        let mut b = Vec::new();
        path(&mut b, p);
        self.status_request(if dir { 15 } else { 13 }, &b)
    }
    fn open(&mut self, p: &str, flags: u32) -> io::Result<Vec<u8>> {
        let mut b = Vec::new();
        path(&mut b, p);
        b.extend_from_slice(&flags.to_be_bytes());
        b.extend_from_slice(&0u32.to_be_bytes());
        self.handle(3, &b)
    }
    /// Pipeline up to 2MiB so throughput isn't limited to one 32KiB packet per RTT.
    pub fn download(
        &mut self,
        p: &str,
        size: u64,
        out: &mut impl Write,
        control: &Control,
        mut progress: impl FnMut(u64),
    ) -> io::Result<()> {
        let h = self.open(p, 1)?;
        let mut offset = 0;
        let mut done = 0;
        while offset < size {
            control.check()?;
            let mut pending = Vec::new();
            for _ in 0..WINDOW {
                if offset >= size {
                    break;
                }
                let n = (size - offset).min(CHUNK as u64) as u32;
                let mut b = Vec::new();
                string(&mut b, &h);
                b.extend_from_slice(&offset.to_be_bytes());
                b.extend_from_slice(&n.to_be_bytes());
                pending.push((self.send(5, &b)?, offset, n));
                offset += n as u64;
            }
            let mut chunks = std::collections::HashMap::new();
            for _ in 0..pending.len() {
                let mut r = self.packet()?;
                if r.t != 103 {
                    if r.t == 101 {
                        r.status()?;
                    }
                    return Err(err("파일이 전송 중 바뀌었거나 읽을 수 없습니다"));
                }
                let id = r.id;
                if chunks.insert(id, r.bytes()?).is_some() {
                    return Err(err("중복 SFTP 응답"));
                }
            }
            for (id, at, n) in pending {
                let mut data = chunks.remove(&id).ok_or_else(|| err("SFTP 응답 누락"))?;
                let mut consumed = 0;
                loop {
                    if data.is_empty() || data.len() > n as usize - consumed {
                        return Err(err(
                            "SFTP 읽기 크기 오류: 전송 중 파일이 바뀌었을 수 있습니다",
                        ));
                    }
                    out.write_all(&data)?;
                    consumed += data.len();
                    done += data.len() as u64;
                    progress(done);
                    if consumed == n as usize {
                        break;
                    }
                    control.check()?;
                    let mut b = Vec::new();
                    string(&mut b, &h);
                    b.extend_from_slice(&(at + consumed as u64).to_be_bytes());
                    b.extend_from_slice(&(n - consumed as u32).to_be_bytes());
                    let mut r = self.request(5, &b)?;
                    if r.t != 103 {
                        return Err(err("전송 중 파일이 바뀌었습니다"));
                    }
                    data = r.bytes()?;
                }
            }
        }
        self.close(&h)
    }
    /// Stream a file to another device with bounded read/write windows. The caller owns
    /// the destination staging name and commits it only after source/size verification.
    pub fn relay_to(&mut self, from: &str, size: u64, dst: &mut Sftp, to: &str, control: &Control, progress: impl FnMut(u64)) -> io::Result<u64> {
        let handle = dst.open(to, 2 | 8 | 16 | 32)?;
        let mut sink = RelayWriter { dst, handle, pending: Vec::new(), offset: 0, control, progress };
        let copied = self.download(from, size, &mut sink, control, |_| {});
        // Drain queued replies before CLOSE / cleanup while the transport is available.
        // Cancellation kills both sessions; the transfer then reconnects to discard staging.
        let flushed = sink.flush();
        let closed = sink.dst.close(&sink.handle);
        copied.and(flushed).and(closed)?;
        Ok(sink.offset)
    }
    pub fn upload(
        &mut self,
        p: &str,
        input: &mut impl Read,
        control: &Control,
        mut progress: impl FnMut(u64),
    ) -> io::Result<u64> {
        let h = self.open(p, 2 | 8 | 16 | 32)?;
        let mut offset = 0u64;
        let mut eof = false;
        while !eof {
            control.check()?;
            let mut pending = Vec::new();
            for _ in 0..WINDOW {
                let mut data = vec![0; CHUNK];
                let mut n = 0;
                while n < data.len() {
                    let got = input.read(&mut data[n..])?;
                    if got == 0 {
                        eof = true;
                        break;
                    }
                    n += got;
                }
                if n == 0 {
                    break;
                }
                data.truncate(n);
                let mut b = Vec::new();
                string(&mut b, &h);
                b.extend_from_slice(&offset.to_be_bytes());
                string(&mut b, &data);
                pending.push(self.send(6, &b)?);
                offset += n as u64;
            }
            for _ in 0..pending.len() {
                let mut r = self.packet()?;
                let i = pending
                    .iter()
                    .position(|&id| id == r.id)
                    .ok_or_else(|| err("SFTP 쓰기 응답 오류"))?;
                pending.remove(i);
                if r.status()? != 0 {
                    return Err(err("파일 쓰기 실패"));
                }
            }
            progress(offset);
        }
        self.close(&h)?;
        Ok(offset)
    }
    /// SETSTAT with permissions only (bits & 0o777: no setuid, setgid or sticky).
    pub fn set_mode(&mut self, p: &str, mode: u32) -> io::Result<()> {
        let mut b = Vec::new();
        path(&mut b, p);
        b.extend_from_slice(&4u32.to_be_bytes());
        b.extend_from_slice(&(mode & 0o777).to_be_bytes());
        self.status_request(9, &b)
    }
    pub fn set_time(&mut self, p: &str, modified: u64) -> io::Result<()> {
        let mut b = Vec::new();
        path(&mut b, p);
        b.extend_from_slice(&8u32.to_be_bytes());
        b.extend_from_slice(&(modified as u32).to_be_bytes());
        b.extend_from_slice(&(modified as u32).to_be_bytes());
        self.status_request(9, &b)
    }
}
#[cfg(test)]
mod tests {
    use super::*;

    /// A server that answers out of order (as SFTP allows): the first three READDIRs come back
    /// reversed (two NAME batches and an "end"), and two CLOSE replies come back reversed.
    /// Every entry is kept and later requests stay in step.
    const SHUFFLING_SERVER: &str = r#"
import struct, sys
i, o = sys.stdin.buffer, sys.stdout.buffer
def read():
    n = i.read(4)
    if len(n) < 4: sys.exit(0)
    b = i.read(struct.unpack(">I", n)[0]); return b[0], struct.unpack(">I", b[1:5])[0] if len(b) >= 5 else 0, b
def send(t, id, body=b""):
    p = bytes([t]) + struct.pack(">I", id) + body; o.write(struct.pack(">I", len(p)) + p); o.flush()
def s(x): x = x.encode(); return struct.pack(">I", len(x)) + x
def names(ns): return struct.pack(">I", len(ns)) + b"".join(s(n) + s(n) + struct.pack(">I", 0) for n in ns)
def status(code): return struct.pack(">I", code) + s("") + s("")
t, _, b = read(); p = struct.pack(">I", 3); o.write(struct.pack(">I", len(p) + 1) + bytes([2]) + p); o.flush()
readdirs, closes, batches = [], [], [["a", "b"], ["c"]]
while True:
    t, id, b = read()
    if t == 11: send(102, id, s("h")); readdirs, batches = [], [["a", "b"], ["c"]]
    elif t == 12:
        readdirs.append(id)
        if len(readdirs) == 3:
            answers = [(104, names(batches[k])) if k < len(batches) else (101, status(1)) for k in range(3)]
            for rid, (rt, body) in reversed(list(zip(readdirs, answers))): send(rt, rid, body)
        elif len(readdirs) > 3: send(101, id, status(1))
    elif t == 4:
        closes.append(id)
        if len(closes) == 2:
            for cid in reversed(closes): send(101, cid, status(0))
            closes = []
    elif t in (7, 17): send(105, id, struct.pack(">IQ", 1, 5))
    else: send(101, id, status(8))
"#;
    #[test]
    fn listing_survives_out_of_order_replies() {
        let Ok(child) = std::process::Command::new("python3").args(["-c", SHUFFLING_SERVER]).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::null()).spawn() else {
            return;
        };
        let control = Control::default();
        let mut s = Sftp::start(child, &control).unwrap();
        let mut first: Vec<String> = s.list("/x").unwrap().into_iter().map(|e| e.name).collect();
        first.sort();
        assert_eq!(first, ["a", "b", "c"]);
        assert_eq!(s.list("/y").unwrap().len(), 3);
        assert_eq!(s.stat("/x/a").unwrap().unwrap().size, 5, "stat after reversed CLOSE replies");
        assert_eq!(s.stat("/x/b").unwrap().unwrap().size, 5);
    }

    /// A folder of several READDIR batches lists whole with requests sent ahead, a missing
    /// folder is refused, and the session stays in step afterwards (the CLOSE replies are
    /// dropped when they arrive).
    #[test]
    fn listing_ahead_stays_in_step() {
        let server = std::env::var_os("ITDA_SFTP_SERVER").map(std::path::PathBuf::from).unwrap_or_else(|| "/usr/libexec/sftp-server".into());
        if !server.is_file() {
            return;
        }
        let root = std::env::temp_dir().join(format!("itda-sftp-list-{}", crate::store::new_id()));
        std::fs::create_dir(&root).unwrap();
        for i in 0..450 {
            std::fs::write(root.join(format!("파일 {i:03}.txt")), b"x").unwrap();
        }
        let control = Control::default();
        let child = std::process::Command::new(server).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::null()).spawn().unwrap();
        let mut s = Sftp::start(child, &control).unwrap();
        let dir = root.to_str().unwrap();
        for _ in 0..3 {
            assert_eq!(s.list(dir).unwrap().len(), 450);
        }
        assert!(s.list(&format!("{dir}/없는 폴더")).is_err());
        assert_eq!(s.stat(&format!("{dir}/파일 007.txt")).unwrap().unwrap().size, 1);
        assert_eq!(s.list(dir).unwrap().len(), 450);
        std::fs::remove_dir_all(&root).unwrap();
    }
    #[test]
    fn real_server_relay_streams_and_cancels() {
        let server = std::env::var_os("ITDA_SFTP_SERVER").map(std::path::PathBuf::from).unwrap_or_else(|| "/usr/libexec/sftp-server".into());
        if !server.is_file() { return; }
        let root = std::env::temp_dir().join(format!("itda-relay-{}", crate::store::new_id()));
        std::fs::create_dir(&root).unwrap();
        let from = root.join("원본.bin").to_string_lossy().into_owned();
        let to = root.join("대상.partial").to_string_lossy().into_owned();
        let bytes: Vec<u8> = (0..(5 * 1024 * 1024 + 13)).map(|i| (i % 251) as u8).collect();
        std::fs::write(&from, &bytes).unwrap();
        let session = |control: &Control| Sftp::start(std::process::Command::new(&server).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::null()).spawn().unwrap(), control).unwrap();
        let control = Control::default();
        let mut src = session(&control);
        let mut dst = session(&control);
        let mut progress = Vec::new();
        assert_eq!(src.relay_to(&from, bytes.len() as u64, &mut dst, &to, &control, |n| progress.push(n)).unwrap(), bytes.len() as u64);
        assert_eq!(std::fs::read(&to).unwrap(), bytes);
        assert!(progress.windows(2).all(|n| n[0] <= n[1]));
        assert_eq!(progress.last(), Some(&(bytes.len() as u64)));
        assert!(progress.len() >= 3, "relay only reported after buffering the whole file");
        // Exclusive staging cannot overwrite a file already present at that name.
        assert!(src.relay_to(&from, bytes.len() as u64, &mut dst, &to, &control, |_| {}).is_err());
        assert_eq!(std::fs::read(&to).unwrap(), bytes);
        drop((src, dst));
        // Cancel while bytes are flowing; it must neither finish nor alter the source.
        let control = Control::default();
        let mut src = session(&control);
        let mut dst = session(&control);
        let cancelled = root.join("cancelled.partial").to_string_lossy().into_owned();
        let result = src.relay_to(&from, bytes.len() as u64, &mut dst, &cancelled, &control, |n| { if n > 0 { control.stop(); } });
        assert!(result.is_err());
        assert!(std::fs::metadata(&cancelled).unwrap().len() < bytes.len() as u64);
        assert_eq!(std::fs::read(&from).unwrap(), bytes);
        drop((src, dst));
        #[cfg(unix)]
        {
            // A real server returns WRITE failures once this child's file-size limit
            // is reached. Its following replies must still be drained for cleanup.
            let control = Control::default();
            let mut src = session(&control);
            let child = std::process::Command::new("/bin/sh")
                .args(["-c", "trap '' XFSZ; ulimit -f 1024; exec \"$1\"", "itda-limited-sftp"])
                .arg(&server).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::null()).spawn().unwrap();
            let mut dst = Sftp::start(child, &control).unwrap();
            let rejected = root.join("rejected.partial").to_string_lossy().into_owned();
            assert!(src.relay_to(&from, bytes.len() as u64, &mut dst, &rejected, &control, |_| {}).is_err());
            assert!(dst.stat(&rejected).unwrap().unwrap().size < bytes.len() as u64);
            dst.remove(&rejected, false).unwrap();
            assert!(!std::path::Path::new(&rejected).exists());
        }
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn real_server_roundtrip() {
        let server = std::env::var_os("ITDA_SFTP_SERVER")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| "/usr/libexec/sftp-server".into());
        if !server.is_file() {
            return;
        }
        let root = std::env::temp_dir().join(format!("itda-sftp-{}", crate::store::new_id()));
        std::fs::create_dir(&root).unwrap();
        let control = Control::default();
        let child = std::process::Command::new(server)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let mut s = Sftp::start(child, &control).unwrap();
        let p = root.join("한글 \"이름\".bin").to_str().unwrap().to_string();
        let bytes: Vec<u8> = (0..(2 << 20)).map(|i| (i % 251) as u8).collect();
        assert_eq!(
            s.upload(&p, &mut &bytes[..], &control, |_| {}).unwrap(),
            bytes.len() as u64
        );
        let attrs = s.stat(&p).unwrap().unwrap();
        let mut got = Vec::new();
        s.download(&p, attrs.size, &mut got, &control, |_| {})
            .unwrap();
        assert_eq!(got, bytes);
        assert_eq!(s.list(root.to_str().unwrap()).unwrap().len(), 1);
        let to = format!("{p}.renamed");
        s.rename(&p, &to, false).unwrap();
        assert!(s.stat(&p).unwrap().is_none());
        // A case-only rename (refused by a case-insensitive server's plain rename).
        let upper = format!("{p}.RENAMED");
        s.rename(&to, &upper, false).unwrap();
        let names: Vec<String> = s.list(root.to_str().unwrap()).unwrap().into_iter().map(|e| e.name).collect();
        assert_eq!(names, ["한글 \"이름\".bin.RENAMED"]);
        s.rename(&upper, &to, false).unwrap();
        // Permissions: ordinary bits set and reported back, special bits never.
        std::fs::write(root.join("tool.sh"), b"#!/bin/sh\n").unwrap();
        let tool = root.join("tool.sh").to_str().unwrap().to_string();
        s.set_mode(&tool, 0o4755).unwrap();
        assert_eq!(s.stat(&tool).unwrap().unwrap().mode, Some(0o755));
        std::fs::remove_file(&tool).unwrap();
        s.remove(&to, false).unwrap();
        // One name the client can't use ("Icon\r") is left out, the rest still listed.
        std::fs::write(root.join("ok.txt"), b"x").unwrap();
        std::fs::write(root.join("Icon\r"), b"").unwrap();
        let names: Vec<String> = s.list(root.to_str().unwrap()).unwrap().into_iter().map(|e| e.name).collect();
        assert_eq!(names, ["ok.txt"]);
        std::fs::remove_file(root.join("ok.txt")).unwrap();
        std::fs::remove_file(root.join("Icon\r")).unwrap();
        drop(s);
        std::fs::remove_dir(root).unwrap();
    }
}
