//! RFB handshake: version, security (None / VNC / Apple ARD), init.

use std::io::{self, BufReader, Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::sync::atomic::{AtomicU64, Ordering::Relaxed};
use std::sync::OnceLock;
use std::time::Duration;

/// 32bpp, depth 24, little-endian, R<<16 G<<8 B — identical to a Windows BGRX DIB,
/// and lets ZRLE use 3-byte CPIXELs.
pub const PIXEL_FORMAT: [u8; 16] = [32, 24, 0, 1, 0, 255, 0, 255, 0, 255, 16, 8, 0, 0, 0, 0];

pub const ENC_RAW: i32 = 0;
pub const ENC_COPYRECT: i32 = 1;
pub const ENC_ZRLE: i32 = 16;
pub const ENC_CURSOR: i32 = -239;
pub const ENC_DESKTOP_SIZE: i32 = -223;
pub const ENC_EXT_DESKTOP_SIZE: i32 = -308;
/// Extended Clipboard pseudo-encoding (handled in clip.rs).
pub const ENC_EXT_CLIPBOARD: i32 = 0xc0a1_e5ce_u32 as i32;

/// Total bytes received from the server (for the stats display).
pub static RX_BYTES: AtomicU64 = AtomicU64::new(0);

/// Socket reader that counts received bytes.
pub struct Rx(pub TcpStream);

impl Read for Rx {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let n = self.0.read(buf)?;
        RX_BYTES.fetch_add(n as u64, Relaxed);
        Ok(n)
    }
}

pub struct Session {
    pub reader: BufReader<Rx>,
    pub writer: TcpStream,
    pub width: u16,
    pub height: u16,
    pub name: String,
}

/// Empty user = not given (prefer VNC password auth).
#[derive(Clone)]
pub struct Creds {
    pub user: String,
    pub pass: String,
}

/// "host" -> "host:5900", keeps explicit ports and bracketed IPv6.
pub fn addr(host: &str) -> String {
    match host.matches(':').count() {
        0 => format!("{host}:5900"),
        1 => host.to_string(),
        _ if host.starts_with('[') && host.contains("]:") => host.to_string(),
        _ => format!("[{host}]:5900"),
    }
}

pub fn err<T>(msg: impl Into<String>) -> io::Result<T> {
    Err(io::Error::new(io::ErrorKind::Other, msg.into()))
}

/// Errors that retrying won't fix (bad credentials): reconnect stops on these.
pub fn denied(msg: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::PermissionDenied, msg.into())
}

pub fn rd_u8(r: &mut impl Read) -> io::Result<u8> {
    let mut b = [0; 1];
    r.read_exact(&mut b)?;
    Ok(b[0])
}
pub fn rd_u16(r: &mut impl Read) -> io::Result<u16> {
    let mut b = [0; 2];
    r.read_exact(&mut b)?;
    Ok(u16::from_be_bytes(b))
}
pub fn rd_u32(r: &mut impl Read) -> io::Result<u32> {
    let mut b = [0; 4];
    r.read_exact(&mut b)?;
    Ok(u32::from_be_bytes(b))
}
pub fn rd_vec(r: &mut impl Read, n: usize) -> io::Result<Vec<u8>> {
    let mut v = vec![0; n];
    r.read_exact(&mut v)?;
    Ok(v)
}
fn rd_reason(r: &mut impl Read) -> io::Result<String> {
    let n = rd_u32(r)? as usize;
    Ok(String::from_utf8_lossy(&rd_vec(r, n.min(1 << 16))?).into_owned())
}

/// Connection progress callback ("resolve", "tcp", "auth", "init"), set by the session process.
pub static PROGRESS: OnceLock<fn(&str)> = OnceLock::new();

fn progress(stage: &str) {
    if let Some(f) = PROGRESS.get() {
        f(stage);
    }
}

pub fn connect(addr: &str, creds: &Creds) -> io::Result<Session> {
    crate::log!("connect {addr}");
    let r = handshake(addr, creds);
    if let Err(e) = &r {
        crate::log!("handshake failed: {e} [{}]", classify(e));
    }
    r
}

/// Coarse error category, for user-facing explanations.
pub fn classify(e: &io::Error) -> &'static str {
    use io::ErrorKind::*;
    let msg = e.to_string();
    match (e.kind(), e.raw_os_error()) {
        _ if msg.contains("SSH 호스트 키") => "ssh_hostkey",
        _ if msg.contains("SSH 로그인 실패") => "ssh_auth",
        _ if msg.contains("원격 로그인(SSH)") => "ssh_refused",
        _ if msg.contains("ssh.exe 없음") => "ssh_missing",
        _ if msg.contains("화면 공유가 꺼져") => "refused",
        _ if msg.contains("declined") => "declined",
        (PermissionDenied, _) if msg.contains("사용자 이름을 입력") => "need_user",
        (PermissionDenied, _) => "auth",
        (_, Some(11001 | 11002 | 11004)) => "dns",
        _ if msg.contains("호스트를 찾을 수 없습니다") => "dns",
        (ConnectionRefused, _) => "refused",
        (TimedOut | WouldBlock, _) => "timeout",
        (_, Some(10051 | 10065)) => "unreachable",
        (ConnectionReset | ConnectionAborted | UnexpectedEof, _) => "reset",
        _ if msg.contains("VNC 서버가 아닙니다") => "not_vnc",
        _ => "other",
    }
}

fn authenticate(addr: &str, creds: &Creds) -> io::Result<Raw> {
    progress("resolve");
    let addrs: Vec<_> = addr.to_socket_addrs()?.collect();
    if addrs.is_empty() {
        return err("호스트를 찾을 수 없습니다");
    }
    // .local names often resolve to both IPv6 link-local and IPv4: try each.
    progress("tcp");
    let per_try = Duration::from_secs(if addrs.len() > 1 { 5 } else { 10 });
    let mut last = None;
    let mut sock = None;
    for sa in &addrs {
        match TcpStream::connect_timeout(sa, per_try) {
            Ok(s) => {
                sock = Some(s);
                break;
            }
            Err(e) => {
                crate::log!("tcp {sa}: {e}");
                last = Some(e);
            }
        }
    }
    let Some(s) = sock else { return Err(last.unwrap()) };
    s.set_nodelay(true)?;
    // A server that accepts but never speaks shouldn't hang the connect forever.
    s.set_read_timeout(Some(Duration::from_secs(20)))?;
    let mut w = s.try_clone()?;
    let mut r = BufReader::with_capacity(1 << 20, Rx(s));

    // ProtocolVersion. macOS announces 3.889; we speak 3.8.
    let mut v = [0u8; 12];
    r.read_exact(&mut v)?;
    if &v[..4] != b"RFB " {
        return err("VNC 서버가 아닙니다");
    }
    let minor: u32 = std::str::from_utf8(&v[8..11]).ok().and_then(|s| s.parse().ok()).unwrap_or(3);
    let minor = match minor {
        8.. => 8,
        7 => 7,
        _ => 3,
    };
    crate::log!("server version {:?} -> 3.{minor}", String::from_utf8_lossy(&v).trim_end());
    w.write_all(format!("RFB 003.00{minor}\n").as_bytes())?;

    let sec = if minor == 3 {
        match rd_u32(&mut r)? {
            0 => return err(format!("server refused: {}", rd_reason(&mut r)?)),
            t => t as u8,
        }
    } else {
        let n = rd_u8(&mut r)? as usize;
        if n == 0 {
            return err(format!("server refused: {}", rd_reason(&mut r)?));
        }
        let types = rd_vec(&mut r, n)?;
        let has = |t| types.contains(&t);
        let pick = if has(30) && !creds.user.is_empty() {
            30
        } else if has(2) {
            2
        } else if has(30) {
            return Err(denied("이 Mac은 계정 로그인이 필요합니다 — 사용자 이름을 입력하세요"));
        } else if has(1) {
            1
        } else {
            return err(format!("no supported security type (server offers {types:?})"));
        };
        crate::log!("security types {types:?} -> {pick}");
        w.write_all(&[pick])?;
        pick
    };

    progress("auth");
    match sec {
        1 => {}
        2 => vnc_auth(&mut r, &mut w, &creds.pass)?,
        30 => ard_auth(&mut r, &mut w, &creds.user, &creds.pass)?,
        t => return err(format!("unsupported security type {t}")),
    }
    if sec != 1 || minor == 8 {
        if rd_u32(&mut r)? != 0 {
            let why = if minor == 8 { rd_reason(&mut r).unwrap_or_default() } else { String::new() };
            return Err(denied(format!("인증 실패 — 사용자 이름/비밀번호를 확인하세요 {why}")));
        }
    }

    // ClientInit (shared) / ServerInit
    progress("init");
    w.write_all(&[1])?;
    let width = rd_u16(&mut r)?;
    let height = rd_u16(&mut r)?;
    let pf = rd_vec(&mut r, 16)?;
    let n = rd_u32(&mut r)? as usize;
    let name_bytes = rd_vec(&mut r, n)?;
    let name = String::from_utf8_lossy(&name_bytes).into_owned();
    crate::log!("ServerInit {width}x{height} pf={pf:02x?} name={name:?}");
    let mut server_init = Vec::with_capacity(24 + n);
    server_init.extend_from_slice(&width.to_be_bytes());
    server_init.extend_from_slice(&height.to_be_bytes());
    server_init.extend_from_slice(&pf);
    server_init.extend_from_slice(&(n as u32).to_be_bytes());
    server_init.extend_from_slice(&name_bytes);
    Ok(Raw { reader: r, writer: w, width, height, name, server_init })
}

/// An authenticated connection right after ServerInit, before any client setup.
pub struct Raw {
    pub reader: BufReader<Rx>,
    pub writer: TcpStream,
    pub width: u16,
    pub height: u16,
    pub name: String,
    /// The ServerInit message as received (to replay to a proxied client).
    pub server_init: Vec<u8>,
}

/// Authenticate and stop after ServerInit (the web gateway relays from there).
pub fn open(addr: &str, creds: &Creds) -> io::Result<Raw> {
    crate::log!("open {addr}");
    let r = authenticate(addr, creds);
    if let Err(e) = &r {
        crate::log!("handshake failed: {e} [{}]", classify(e));
    }
    r
}

fn handshake(addr: &str, creds: &Creds) -> io::Result<Session> {
    let Raw { reader: r, writer: mut w, width, height, name, .. } = authenticate(addr, creds)?;
    // SetPixelFormat + SetEncodings + first full update request, in one packet.
    let mut m = vec![0u8, 0, 0, 0];
    m.extend_from_slice(&PIXEL_FORMAT);
    let encs = [ENC_ZRLE, ENC_COPYRECT, ENC_RAW, ENC_CURSOR, ENC_EXT_DESKTOP_SIZE, ENC_DESKTOP_SIZE, ENC_EXT_CLIPBOARD];
    m.extend_from_slice(&[2, 0]);
    m.extend_from_slice(&(encs.len() as u16).to_be_bytes());
    for e in encs {
        m.extend_from_slice(&e.to_be_bytes());
    }
    m.extend_from_slice(&update_request(false, 0, 0, width, height));
    w.write_all(&m)?;

    Ok(Session { reader: r, writer: w, width, height, name })
}

pub fn update_request(incremental: bool, x: u16, y: u16, w: u16, h: u16) -> [u8; 10] {
    let [x0, x1] = x.to_be_bytes();
    let [y0, y1] = y.to_be_bytes();
    let [w0, w1] = w.to_be_bytes();
    let [h0, h1] = h.to_be_bytes();
    [3, incremental as u8, x0, x1, y0, y1, w0, w1, h0, h1]
}

fn vnc_auth(r: &mut impl Read, w: &mut impl Write, pass: &str) -> io::Result<()> {
    let mut ch = [0u8; 16];
    r.read_exact(&mut ch)?;
    w.write_all(&vnc_response(&ch, pass))
}

/// VNC authentication: DES-encrypt the challenge with the (bit-reversed, 8-byte) password.
/// The server side (`share`) computes the same and compares.
pub fn vnc_response(challenge: &[u8; 16], pass: &str) -> [u8; 16] {
    use des::cipher::{generic_array::GenericArray, BlockEncrypt, KeyInit};
    let mut out = *challenge;
    let mut key = [0u8; 8];
    for (k, b) in key.iter_mut().zip(pass.bytes()) {
        *k = b.reverse_bits();
    }
    let c = des::Des::new_from_slice(&key).unwrap();
    for blk in out.chunks_exact_mut(8) {
        c.encrypt_block(GenericArray::from_mut_slice(blk));
    }
    out
}

/// Apple Remote Desktop auth (type 30): DH key agreement, then
/// AES-128-ECB(MD5(shared secret)) over 64-byte username + 64-byte password.
fn ard_auth(r: &mut impl Read, w: &mut impl Write, user: &str, pass: &str) -> io::Result<()> {
    use aes::cipher::{generic_array::GenericArray, BlockEncrypt, KeyInit};
    use md5::{Digest, Md5};
    use num_bigint::BigUint;

    let g = BigUint::from(rd_u16(r)?);
    let klen = rd_u16(r)? as usize;
    let p = BigUint::from_bytes_be(&rd_vec(r, klen)?);
    let server_pub = BigUint::from_bytes_be(&rd_vec(r, klen)?);

    let mut xb = vec![0u8; klen];
    getrandom::getrandom(&mut xb).map_err(io::Error::other)?;
    let x = BigUint::from_bytes_be(&xb);
    let client_pub = g.modpow(&x, &p);
    let shared = server_pub.modpow(&x, &p);
    let key = Md5::digest(be_padded(&shared, klen));

    let mut cred = [0u8; 128];
    getrandom::getrandom(&mut cred).map_err(io::Error::other)?;
    put_cstr(&mut cred[..64], user);
    put_cstr(&mut cred[64..], pass);
    let aes = aes::Aes128::new(&key);
    for blk in cred.chunks_exact_mut(16) {
        aes.encrypt_block(GenericArray::from_mut_slice(blk));
    }
    let mut out = cred.to_vec();
    out.extend_from_slice(&be_padded(&client_pub, klen));
    w.write_all(&out)
}

fn be_padded(n: &num_bigint::BigUint, len: usize) -> Vec<u8> {
    let b = n.to_bytes_be();
    let mut v = vec![0; len.saturating_sub(b.len())];
    v.extend_from_slice(&b);
    v
}

fn put_cstr(dst: &mut [u8], s: &str) {
    let n = s.len().min(dst.len() - 1);
    dst[..n].copy_from_slice(&s.as_bytes()[..n]);
    dst[n] = 0;
}
