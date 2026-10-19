//! Continuous Updates against the bundled noVNC in a real browser, over a slow link.
//!
//! `cargo test --release cu_e2e -- --ignored --nocapture` serves http://127.0.0.1:18790/ and
//! exits after two viewer sessions; `scripts/cu-e2e.mjs` opens them (classic, then
//! continuous) in headless Chromium. Each session runs `Demand` + `Encoder` like the servers
//! do, with SERVER_DELAY added to everything the server sends, and prints how many updates
//! the viewer acknowledged (asked again, or answered the fence after them) while the square
//! moved, and how often it captured once the screen stood still.

use crate::demand::{self, Demand, ENC_CONTINUOUS};
use crate::enc::{Encoder, PixelFormat, ENC_ZRLE, TILE};
use crate::rfb::PIXEL_FORMAT;
use crate::ws;
use std::io::{self, BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::mpsc::{channel, Receiver, RecvTimeoutError, Sender};
use std::time::{Duration, Instant};

const W: usize = 640;
const H: usize = 360;
const FRAME: Duration = Duration::from_millis(33);
const SERVER_DELAY: Duration = Duration::from_millis(100);
/// The square moves for MOVING, then the screen stands still until SESSION.
const MOVING: Duration = Duration::from_secs(5);
const SESSION: Duration = Duration::from_secs(8);
const NOVNC: &str = include_str!("../web/viewer/novnc.min.js");

const PAGE: &str = r#"<!doctype html><meta charset=utf-8><div id=screen style="width:640px;height:360px"></div>
<script type=module>
import './novnc.min.js';
const mode = new URLSearchParams(location.search).get('mode');
const rfb = new window.RFB(document.getElementById('screen'), `ws://${location.host}/ws?mode=${mode}`);
rfb.scaleViewport = false;
setTimeout(async () => {
  const c = document.querySelector('#screen canvas');
  const d = c.getContext('2d').getImageData(0, 0, c.width, c.height).data;
  const samples = [];
  for (let y = 3; y < 120; y += 13) for (let x = 5; x < 640; x += 37) {
    const i = (y * c.width + x) * 4;
    samples.push([x, y, d[i] << 16 | d[i + 1] << 8 | d[i + 2]]);
  }
  await fetch('/report', { method: 'POST', body: JSON.stringify({ mode, continuous: rfb._enabledContinuousUpdates, w: c.width, h: c.height, samples }) });
  document.title = 'done';
}, 4500);
</script>"#;

/// Panels in three colours (packed-palette tiles) above, a moving square below y = 150.
fn frame(t: Duration, f: &mut [u32]) {
    for y in 0..H {
        for x in 0..W {
            f[y * W + x] = still(x, y);
        }
    }
    let sx = (t.as_millis() as usize / 4) % (W - 60);
    for y in 150..210 {
        for p in &mut f[y * W + sx..y * W + sx + 60] {
            *p = 0xff4040;
        }
    }
}

fn still(x: usize, y: usize) -> u32 {
    [0x4744b0, 0xf3f3f3, 0x202020][(x / 40 + y / 30) % 3]
}

/// What a session saw: updates sent and acknowledged while moving, captures while still.
#[derive(Default, Debug)]
struct Stats {
    sent: usize,
    acked: usize,
    bytes: usize,
    idle_captures: usize,
    continuous: bool,
}

enum In {
    Format(PixelFormat),
    Encodings(Vec<i32>),
    Request(bool),
    Ext(demand::Ext),
}

/// The browser's WebSocket messages as one byte stream.
struct WsRead {
    rx: ws::Receiver,
    buf: Vec<u8>,
    pos: usize,
}

impl Read for WsRead {
    fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
        while self.pos == self.buf.len() {
            match self.rx.next()? {
                Some(b) => (self.buf, self.pos) = (b, 0),
                None => return Ok(0),
            }
        }
        let n = out.len().min(self.buf.len() - self.pos);
        out[..n].copy_from_slice(&self.buf[self.pos..self.pos + n]);
        self.pos += n;
        Ok(n)
    }
}

fn read_client(mut r: WsRead, tx: Sender<In>) -> io::Result<()> {
    let mut b1 = [0u8];
    loop {
        r.read_exact(&mut b1)?;
        let m = match b1[0] {
            0 => {
                let mut b = [0u8; 19];
                r.read_exact(&mut b)?;
                In::Format(PixelFormat::parse(&b[3..]))
            }
            2 => {
                let mut b = [0u8; 3];
                r.read_exact(&mut b)?;
                let mut e = vec![0u8; u16::from_be_bytes([b[1], b[2]]) as usize * 4];
                r.read_exact(&mut e)?;
                In::Encodings(e.chunks_exact(4).map(|c| i32::from_be_bytes([c[0], c[1], c[2], c[3]])).collect())
            }
            3 => {
                let mut b = [0u8; 9];
                r.read_exact(&mut b)?;
                In::Request(b[0] != 0)
            }
            4 => {
                r.read_exact(&mut [0u8; 7])?;
                continue;
            }
            5 => {
                r.read_exact(&mut [0u8; 5])?;
                continue;
            }
            t @ (demand::MSG_CONTINUOUS | demand::MSG_FENCE) => In::Ext(demand::read(t, &mut r)?),
            t => return Err(io::Error::other(format!("client message {t}"))),
        };
        if tx.send(m).is_err() {
            return Ok(());
        }
    }
}

/// Everything the server writes reaches the browser SERVER_DELAY later, in order.
fn delayed(ws: ws::Sender) -> Sender<Vec<u8>> {
    let (tx, rx) = channel::<(Instant, Vec<u8>)>();
    std::thread::spawn(move || {
        while let Ok((at, b)) = rx.recv() {
            std::thread::sleep(at.saturating_duration_since(Instant::now()));
            if ws.binary(&b).is_err() {
                return;
            }
        }
    });
    let (out, inp) = channel::<Vec<u8>>();
    std::thread::spawn(move || {
        while let Ok(b) = inp.recv() {
            if tx.send((Instant::now() + SERVER_DELAY, b)).is_err() {
                return;
            }
        }
    });
    out
}

fn session(s: TcpStream, continuous: bool) -> io::Result<Stats> {
    let tx = ws::Sender::new(s.try_clone()?);
    let mut r = WsRead { rx: ws::Receiver::new(s.try_clone()?, tx.clone()), buf: Vec::new(), pos: 0 };
    let out = delayed(tx);
    let send = |b: Vec<u8>| out.send(b).map_err(|_| io::Error::other("closed"));
    // RFB 3.8, security None.
    send(b"RFB 003.008\n".to_vec())?;
    r.read_exact(&mut [0u8; 12])?;
    send(vec![1, 1])?;
    r.read_exact(&mut [0u8; 1])?;
    send(vec![0, 0, 0, 0])?;
    r.read_exact(&mut [0u8; 1])?;
    let mut init = Vec::new();
    init.extend_from_slice(&(W as u16).to_be_bytes());
    init.extend_from_slice(&(H as u16).to_be_bytes());
    init.extend_from_slice(&PIXEL_FORMAT);
    init.extend_from_slice(&4u32.to_be_bytes());
    init.extend_from_slice(b"test");
    send(init)?;

    let (itx, irx): (Sender<In>, Receiver<In>) = channel();
    std::thread::spawn(move || read_client(r, itx));
    let mut pf = PixelFormat::parse(&PIXEL_FORMAT);
    let mut zrle = false;
    let mut demand = Demand::default();
    let mut enc = Encoder::new();
    let mut prev: Vec<u32> = Vec::new();
    let mut f = vec![0u32; W * H];
    let mut st = Stats::default();
    let (start, mut last_capture) = (Instant::now(), Instant::now() - FRAME);
    let moving = || start.elapsed() < MOVING;
    while start.elapsed() < SESSION {
        let wait = match demand.due() {
            Some(_) => FRAME.saturating_sub(last_capture.elapsed()),
            None => FRAME,
        };
        let mut next = match irx.recv_timeout(wait.max(Duration::from_millis(1))) {
            Ok(m) => Some(m),
            Err(RecvTimeoutError::Timeout) => None,
            Err(RecvTimeoutError::Disconnected) => break,
        };
        let mut reply = Vec::new();
        while let Some(m) = next {
            match m {
                In::Format(p) => {
                    pf = p;
                    prev.clear();
                }
                In::Encodings(mut e) => {
                    if !continuous {
                        e.retain(|&x| x != ENC_CONTINUOUS);
                    }
                    zrle = e.contains(&ENC_ZRLE);
                    demand.encodings(&e, &mut reply);
                }
                In::Request(incremental) => {
                    // Asking again acknowledges the update before (classic mode).
                    if incremental && moving() && st.sent > 0 && !continuous {
                        st.acked += 1;
                    }
                    demand.request(incremental);
                }
                In::Ext(m) => {
                    match m {
                        demand::Ext::Fence { flags: 0, .. } if moving() => st.acked += 1,
                        demand::Ext::Continuous { on: true, .. } => st.continuous = true,
                        _ => {}
                    }
                    if demand.ext(m, W, H, &mut reply) {
                        prev.clear();
                    }
                }
            }
            next = irx.try_recv().ok();
        }
        if !reply.is_empty() {
            send(reply)?;
        }
        assert!(!demand.stalled());
        let Some(incremental) = demand.due() else { continue };
        if last_capture.elapsed() < FRAME {
            continue;
        }
        last_capture = Instant::now();
        if !moving() {
            st.idle_captures += 1;
        }
        frame(start.elapsed().min(MOVING), &mut f);
        let full = !incremental || prev.len() != f.len();
        let mut body = Vec::new();
        let mut rects = 0u16;
        for ty in (0..H).step_by(TILE) {
            let th = TILE.min(H - ty);
            let mut tx = 0;
            while tx < W {
                let dirty = |x: usize| {
                    let tw = TILE.min(W - x);
                    demand.covers(x, ty, tw, th) && (full || (ty..ty + th).any(|y| f[y * W + x..y * W + x + tw] != prev[y * W + x..y * W + x + tw]))
                };
                if !dirty(tx) {
                    tx += TILE;
                    continue;
                }
                let s = tx;
                while tx < W && dirty(tx) {
                    tx += TILE;
                }
                enc.rect(&mut body, &f, W, s, ty, tx.min(W) - s, th, &pf, zrle)?;
                rects += 1;
            }
        }
        if rects == 0 {
            continue;
        }
        let mut msg = vec![0u8, 0];
        msg.extend_from_slice(&rects.to_be_bytes());
        msg.extend_from_slice(&body);
        demand.sent(msg.len(), &mut msg);
        st.bytes += msg.len();
        st.sent += 1;
        send(msg)?;
        prev.clone_from(&f);
    }
    Ok(st)
}

#[test]
#[ignore]
fn cu_e2e() {
    let l = TcpListener::bind("127.0.0.1:18790").unwrap();
    let mut sessions = 0;
    let mut stats = Vec::new();
    for c in l.incoming() {
        let mut c = c.unwrap();
        let mut r = BufReader::new(c.try_clone().unwrap());
        let mut line = String::new();
        r.read_line(&mut line).unwrap();
        let path = line.split(' ').nth(1).unwrap_or("/").to_string();
        let mut key = String::new();
        let mut len = 0;
        loop {
            let mut h = String::new();
            r.read_line(&mut h).unwrap();
            if h.trim().is_empty() {
                break;
            }
            let (k, v) = h.split_once(':').unwrap_or(("", ""));
            match k.trim().to_ascii_lowercase().as_str() {
                "sec-websocket-key" => key = v.trim().to_string(),
                "content-length" => len = v.trim().parse().unwrap_or(0),
                _ => {}
            }
        }
        let reply = |c: &mut TcpStream, mime: &str, body: &[u8]| {
            let _ = write!(c, "HTTP/1.1 200 OK\r\nContent-Type: {mime}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", body.len());
            let _ = c.write_all(body);
        };
        if path.starts_with("/ws") {
            let continuous = path.contains("mode=cu");
            write!(c, "HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Accept: {}\r\n\r\n", ws::accept_key(&key)).unwrap();
            c.set_nodelay(true).unwrap();
            let st = session(c, continuous).unwrap();
            println!("session {}: {st:?}", if continuous { "continuous" } else { "classic" });
            stats.push((continuous, st));
        } else if path == "/report" {
            let mut body = vec![0u8; len];
            r.read_exact(&mut body).unwrap();
            let v: serde_json::Value = serde_json::from_slice(&body).unwrap();
            let bad: Vec<_> = v["samples"]
                .as_array()
                .unwrap()
                .iter()
                .filter(|s| s[2].as_u64().unwrap() as u32 != still(s[0].as_u64().unwrap() as usize, s[1].as_u64().unwrap() as usize))
                .collect();
            println!("report {}: viewer continuous={} canvas {}x{}, {} samples, {} wrong {:?}", v["mode"], v["continuous"], v["w"], v["h"], v["samples"].as_array().unwrap().len(), bad.len(), &bad[..bad.len().min(5)]);
            assert!(bad.is_empty(), "pixels drawn wrong");
            assert_eq!(v["continuous"].as_bool(), Some(v["mode"] == "cu"));
            reply(&mut c, "text/plain", b"ok");
            sessions += 1;
            if sessions == 2 {
                break;
            }
        } else if path.starts_with("/novnc.min.js") {
            reply(&mut c, "text/javascript", NOVNC.as_bytes());
        } else {
            reply(&mut c, "text/html", PAGE.as_bytes());
        }
    }
    let still = (SESSION - MOVING).as_secs_f64();
    for (continuous, st) in &stats {
        let fps = st.acked as f64 / MOVING.as_secs_f64();
        let cps = st.idle_captures as f64 / still;
        println!("{:>10}: {fps:.1} updates/s acknowledged while moving, {} sent, {} KB; {cps:.1} captures/s while still", if *continuous { "continuous" } else { "classic" }, st.sent, st.bytes / 1024);
        assert!(cps < 1.2 / FRAME.as_secs_f64(), "a still screen must be captured at most once a FRAME");
    }
    let fps = |c: bool| stats.iter().find(|s| s.0 == c).map_or(0.0, |s| s.1.acked as f64);
    assert!(fps(true) > fps(false) * 1.5, "continuous updates should beat request/answer over a {SERVER_DELAY:?} link");
}
