//! Minimal RFC 6455 WebSocket server side (binary frames only) plus the SHA-1/base64
//! needed for the handshake.

use std::io::{self, Read, Write};
use std::net::TcpStream;
use std::sync::{Arc, Mutex};

const GUID: &str = "258EAFA5-E914-47DA-95CA-C5AB0DC85B11";

pub fn sha1(data: &[u8]) -> [u8; 20] {
    let mut h: [u32; 5] = [0x67452301, 0xEFCDAB89, 0x98BADCFE, 0x10325476, 0xC3D2E1F0];
    let mut msg = data.to_vec();
    let bits = (data.len() as u64).wrapping_mul(8);
    msg.push(0x80);
    while msg.len() % 64 != 56 {
        msg.push(0);
    }
    msg.extend_from_slice(&bits.to_be_bytes());
    for block in msg.chunks_exact(64) {
        let mut w = [0u32; 80];
        for i in 0..16 {
            w[i] = u32::from_be_bytes([block[i * 4], block[i * 4 + 1], block[i * 4 + 2], block[i * 4 + 3]]);
        }
        for i in 16..80 {
            w[i] = (w[i - 3] ^ w[i - 8] ^ w[i - 14] ^ w[i - 16]).rotate_left(1);
        }
        let [mut a, mut b, mut c, mut d, mut e] = h;
        for (i, wi) in w.iter().enumerate() {
            let (f, k) = match i {
                0..=19 => ((b & c) | (!b & d), 0x5A827999),
                20..=39 => (b ^ c ^ d, 0x6ED9EBA1),
                40..=59 => ((b & c) | (b & d) | (c & d), 0x8F1BBCDC),
                _ => (b ^ c ^ d, 0xCA62C1D6),
            };
            let t = a.rotate_left(5).wrapping_add(f).wrapping_add(e).wrapping_add(k).wrapping_add(*wi);
            e = d;
            d = c;
            c = b.rotate_left(30);
            b = a;
            a = t;
        }
        for (x, y) in h.iter_mut().zip([a, b, c, d, e]) {
            *x = x.wrapping_add(y);
        }
    }
    let mut out = [0u8; 20];
    for (i, v) in h.iter().enumerate() {
        out[i * 4..i * 4 + 4].copy_from_slice(&v.to_be_bytes());
    }
    out
}

pub fn base64(data: &[u8]) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut s = String::with_capacity((data.len() + 2) / 3 * 4);
    for c in data.chunks(3) {
        let n = (c[0] as u32) << 16 | (*c.get(1).unwrap_or(&0) as u32) << 8 | *c.get(2).unwrap_or(&0) as u32;
        for i in 0..4 {
            if i <= c.len() {
                s.push(T[(n >> (18 - 6 * i) & 63) as usize] as char);
            } else {
                s.push('=');
            }
        }
    }
    s
}

/// Standard base64 (padding optional); None on any other character.
pub fn base64_decode(s: &str) -> Option<Vec<u8>> {
    let val = |c: u8| match c {
        b'A'..=b'Z' => Some(c - b'A'),
        b'a'..=b'z' => Some(c - b'a' + 26),
        b'0'..=b'9' => Some(c - b'0' + 52),
        b'+' => Some(62),
        b'/' => Some(63),
        _ => None,
    };
    let s = s.trim_end_matches('=').as_bytes();
    let mut out = Vec::with_capacity(s.len() * 3 / 4);
    let (mut acc, mut bits) = (0u32, 0);
    for &c in s {
        acc = acc << 6 | val(c)? as u32;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((acc >> bits) as u8);
        }
    }
    Some(out)
}

pub fn accept_key(key: &str) -> String {
    base64(&sha1(format!("{}{GUID}", key.trim()).as_bytes()))
}

/// Writing half, shared by the pump thread and the reader (for pongs and close).
#[derive(Clone)]
pub struct Sender(Arc<Mutex<TcpStream>>);

impl Sender {
    pub fn new(s: TcpStream) -> Sender {
        Sender(Arc::new(Mutex::new(s)))
    }

    fn frame(&self, opcode: u8, data: &[u8]) -> io::Result<()> {
        // Header and payload in one write: two small writes stall on Nagle + delayed ACK.
        let mut hdr = Vec::with_capacity(10 + data.len());
        hdr.push(0x80 | opcode);
        match data.len() {
            n if n < 126 => hdr.push(n as u8),
            n if n < 65536 => {
                hdr.push(126);
                hdr.extend_from_slice(&(n as u16).to_be_bytes());
            }
            n => {
                hdr.push(127);
                hdr.extend_from_slice(&(n as u64).to_be_bytes());
            }
        }
        hdr.extend_from_slice(data);
        self.0.lock().unwrap().write_all(&hdr)
    }

    pub fn binary(&self, data: &[u8]) -> io::Result<()> {
        self.frame(2, data)
    }

    /// Close with an application code (4000–4999) and a short UTF-8 reason.
    pub fn close(&self, code: u16, reason: &str) {
        let mut p = code.to_be_bytes().to_vec();
        // Control frames carry at most 125 bytes: cut the reason on a char boundary.
        let cut = reason.char_indices().map(|(i, c)| i + c.len_utf8()).take_while(|&end| end <= 120).last().unwrap_or(0);
        p.extend_from_slice(&reason.as_bytes()[..cut]);
        let _ = self.frame(8, &p);
        let _ = self.0.lock().unwrap().shutdown(std::net::Shutdown::Both);
    }

    fn pong(&self, data: &[u8]) -> io::Result<()> {
        self.frame(10, data)
    }
}

/// Reading half: yields the payload bytes of data frames as one stream.
pub struct Receiver {
    s: TcpStream,
    tx: Sender,
    buf: Vec<u8>,
    pos: usize,
    closed: bool,
}

impl Receiver {
    pub fn new(s: TcpStream, tx: Sender) -> Receiver {
        Receiver { s, tx, buf: Vec::new(), pos: 0, closed: false }
    }

    /// Next data payload (None when the peer closed).
    pub fn next(&mut self) -> io::Result<Option<Vec<u8>>> {
        loop {
            if self.closed {
                return Ok(None);
            }
            let mut h = [0u8; 2];
            self.s.read_exact(&mut h)?;
            let opcode = h[0] & 0x0f;
            let masked = h[1] & 0x80 != 0;
            let mut len = (h[1] & 0x7f) as u64;
            if len == 126 {
                let mut b = [0u8; 2];
                self.s.read_exact(&mut b)?;
                len = u16::from_be_bytes(b) as u64;
            } else if len == 127 {
                let mut b = [0u8; 8];
                self.s.read_exact(&mut b)?;
                len = u64::from_be_bytes(b);
            }
            if len > 16 << 20 {
                return Err(io::Error::other("websocket frame too large"));
            }
            let mut mask = [0u8; 4];
            if masked {
                self.s.read_exact(&mut mask)?;
            }
            let mut data = vec![0u8; len as usize];
            self.s.read_exact(&mut data)?;
            if masked {
                for (i, b) in data.iter_mut().enumerate() {
                    *b ^= mask[i % 4];
                }
            }
            match opcode {
                0 | 1 | 2 => return Ok(Some(data)),
                8 => {
                    self.closed = true;
                    self.tx.close(1000, "");
                    return Ok(None);
                }
                9 => self.tx.pong(&data)?,
                _ => {}
            }
        }
    }
}

impl Read for Receiver {
    fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
        while self.pos >= self.buf.len() {
            match self.next()? {
                Some(d) => {
                    self.buf = d;
                    self.pos = 0;
                }
                None => return Ok(0),
            }
        }
        let n = out.len().min(self.buf.len() - self.pos);
        out[..n].copy_from_slice(&self.buf[self.pos..self.pos + n]);
        self.pos += n;
        Ok(n)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rfc6455_example_key() {
        assert_eq!(accept_key("dGhlIHNhbXBsZSBub25jZQ=="), "s3pPLMBiTxaQ9kYGzzhZRbK+xOo=");
    }

    #[test]
    fn sha1_and_base64() {
        let hex: String = sha1(b"abc").iter().map(|b| format!("{b:02x}")).collect();
        assert_eq!(hex, "a9993e364706816aba3e25717850c26c9cd0d89d");
        assert_eq!(base64(b"Ma"), "TWE=");
        assert_eq!(base64(b"M"), "TQ==");
        for s in [&b""[..], b"M", b"Ma", b"Man", b"\x89PNG\r\n\x1a\n\0\xff"] {
            assert_eq!(base64_decode(&base64(s)).as_deref(), Some(s));
        }
        assert_eq!(base64_decode("TQ"), Some(b"M".to_vec()));
        assert_eq!(base64_decode("T!=="), None);
    }
}
