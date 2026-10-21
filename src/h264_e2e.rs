//! H.264 (RFB encoding 50) against the bundled noVNC in a real browser, next to ZRLE.
//!
//! `cargo test --release h264_e2e -- --ignored --nocapture` serves http://127.0.0.1:18791/;
//! `scripts/h264-e2e.mjs` opens it twice in Chromium (zrle, then h264). Each session moves
//! a square for MOVING, then stands still; the page samples the canvas at the end. The server
//! answers each update request with the latest frame (classic request/answer, so the rate is
//! what the viewer can take), and reports frames and bytes for both encodings.
use crate::cu_e2e::{read_client, In, WsRead, NOVNC};
use crate::enc::{Encoder, PixelFormat, Update, ENC_ZRLE};
use crate::h264_mac::H264;
use crate::rfb::PIXEL_FORMAT;
use crate::ws;
use std::io::{self, BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::mpsc::{channel, RecvTimeoutError};
use std::time::{Duration, Instant};

pub(crate) const ENC_H264: i32 = 50;
const W: usize = 1920;
const H: usize = 1080;

/// A document scrolling past (dark glyph-like runs on white, coloured headings), a photo-like
/// gradient panel on the right, then standing still once MOVING has passed.
fn frame(t: Duration, f: &mut [u32], _: bool) {
    let off = (t.as_millis() as usize * 3 / 4) % 4000;
    for y in 0..H {
        let dy = y + off;
        let line = dy / 22;
        let ink = dy % 22 < 14;
        for x in 0..W {
            f[y * W + x] = if x >= 1400 {
                let (a, b) = ((x as u32 * 7 + dy as u32 * 3) & 255, (x as u32 ^ dy as u32).wrapping_mul(2654435761) >> 28);
                (a << 16) | (((dy as u32 * 5) & 255) << 8) | ((a / 2 + b * 8) & 255)
            } else if ink && x > 60 && x < 1340 && (x / 9 + line * 7) % 11 != 0 && (x * 31 + dy * 17) % 7 < 4 {
                if line % 9 == 0 { 0x1f5fbf } else { 0x222222 }
            } else {
                0xffffff
            };
        }
    }
}

const MOVING: Duration = Duration::from_secs(5);
const SESSION: Duration = Duration::from_secs(8);
/// How long the picture stands still before its lossless repaint.
const REFINE: Duration = Duration::from_millis(250);

const PAGE: &str = r#"<!doctype html><meta charset=utf-8><div id=screen style="width:1920px;height:1080px"></div>
<script type=module>
import './novnc.min.js';
const mode = new URLSearchParams(location.search).get('mode');
// The h264 session also runs the app viewer's ITDA_HP_TRACE hooks (web/viewer/viewer.js).
if (mode === 'h264') await import('./trace.js');
const rfb = new window.RFB(document.getElementById('screen'), `ws://${location.host}/ws?mode=${mode}`);
rfb.scaleViewport = false;
// Counted independently of the hooks: decoded frames drawn to noVNC's back buffer, and
// copies of that buffer to the visible canvas.
const seen = { videoDraws: 0, screenCopies: 0 };
if (window.viewerTrace) {
  viewerTrace.attach(rfb);
  const d = rfb._display, back = d._drawCtx.drawImage.bind(d._drawCtx), screen = d._targetCtx.drawImage.bind(d._targetCtx);
  d._drawCtx.drawImage = (img, ...a) => { if (typeof VideoFrame !== 'undefined' && img instanceof VideoFrame) seen.videoDraws++; return back(img, ...a); };
  d._targetCtx.drawImage = (img, ...a) => { if (img === d._backbuffer) seen.screenCopies++; return screen(img, ...a); };
}
setTimeout(async () => {
  const c = document.querySelector('#screen canvas');
  const d = c.getContext('2d').getImageData(0, 0, c.width, c.height).data;
  const samples = [];
  for (let y = 3; y < 1080; y += 47) for (let x = 5; x < 1920; x += 53) {
    const i = (y * c.width + x) * 4;
    samples.push([x, y, d[i] << 16 | d[i + 1] << 8 | d[i + 2]]);
  }
  const trace = window.viewerTrace ? { ...viewerTrace.counts, ...seen, reports: (window.traceReports || []).length } : null;
  await fetch('/report', { method: 'POST', body: JSON.stringify({ mode, h264: typeof VideoDecoder !== 'undefined', w: c.width, h: c.height, samples, trace }) });
  document.title = 'done';
}, 7000);
</script>"#;

/// The app viewer's trace hooks, on, reporting into window.traceReports.
fn trace_js() -> String {
    let v = include_str!("../web/viewer/viewer.js");
    let a = v.find("const viewerTrace = (() => {").expect("viewerTrace in viewer.js");
    let b = a + v[a..].find("})();").expect("end of viewerTrace") + 5;
    format!("const TRACE = true;\nconst post = (m) => (window.traceReports ??= []).push(m);\n{}\nwindow.viewerTrace = viewerTrace;\n", &v[a..b])
}

#[derive(Default, Debug)]
struct Stats {
    frames: usize,
    moving_frames: usize,
    bytes: usize,
    key_bytes: usize,
    encode_ms: f64,
    used_h264: bool,
    refines: usize,
}

fn session(s: TcpStream, want_h264: bool) -> io::Result<Stats> {
    let tx = ws::Sender::new(s.try_clone()?);
    let mut r = WsRead { rx: ws::Receiver::new(s.try_clone()?, tx.clone()), buf: Vec::new(), pos: 0 };
    let send = |b: &[u8]| tx.binary(b);
    send(b"RFB 003.008\n")?;
    r.read_exact(&mut [0u8; 12])?;
    send(&[1, 1])?;
    r.read_exact(&mut [0u8; 1])?;
    send(&[0, 0, 0, 0])?;
    r.read_exact(&mut [0u8; 1])?;
    let mut init = Vec::new();
    init.extend_from_slice(&(W as u16).to_be_bytes());
    init.extend_from_slice(&(H as u16).to_be_bytes());
    init.extend_from_slice(&PIXEL_FORMAT);
    init.extend_from_slice(&4u32.to_be_bytes());
    init.extend_from_slice(b"test");
    send(&init)?;
    let (itx, irx) = channel();
    std::thread::spawn(move || read_client(r, itx));
    let (mut pf, mut encs) = (PixelFormat::parse(&PIXEL_FORMAT), Vec::<i32>::new());
    let mut st = Stats::default();
    let (mut enc, mut u, mut prev) = (Encoder::new(), Update::default(), Vec::<u32>::new());
    let mut f = vec![0u32; W * H];
    let mut h264: Option<H264> = None;
    let start = Instant::now();
    let mut pending: Option<bool> = None;
    let (mut last_change, mut exact) = (Instant::now(), Vec::<u32>::new());
    while start.elapsed() < SESSION {
        match irx.recv_timeout(Duration::from_millis(5)) {
            Ok(In::Format(p)) => pf = p,
            Ok(In::Encodings(e)) => encs = e,
            Ok(In::Request(inc)) => pending = Some(pending.map_or(inc, |p| p && inc)),
            Ok(In::Ext(_)) => {}
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => break,
        }
        let Some(incremental) = pending else { continue };
        frame(start.elapsed().min(MOVING), &mut f, false);
        let moving = start.elapsed() < MOVING;
        let video = want_h264 && encs.contains(&ENC_H264);
        if video && incremental && prev == f {
            // Still: once it has stayed still a moment, repaint what changed since the last
            // exact picture losslessly (H.264's 4:2:0 blurs text and colour edges).
            if last_change.elapsed() >= REFINE && exact != f {
                pending = None;
                u.start();
                enc.changes(&mut u, &f, &exact, W, H, exact.len() != f.len(), |_, _, _, _| true, &pf, encs.contains(&ENC_ZRLE))?;
                if u.finish() {
                    st.refines += 1;
                    st.bytes += u.buf.len();
                    send(&u.buf)?;
                }
                exact.clone_from(&f);
            }
            continue;
        }
        if incremental && !moving && prev == f {
            continue; // nothing new: keep the request pending
        }
        last_change = Instant::now();
        pending = None;
        u.start();
        let t = Instant::now();
        if want_h264 && encs.contains(&ENC_H264) {
            let first = h264.is_none();
            let e = h264.get_or_insert_with(|| H264::new(W, H, 20_000_000).expect("encoder"));
            let unit = e.encode(&f, W, first || !incremental).map_err(io::Error::other)?;
            if unit.key {
                st.key_bytes += unit.data.len();
            }
            for v in [0u16, 0, W as u16, H as u16] {
                u.buf.extend_from_slice(&v.to_be_bytes());
            }
            u.buf.extend_from_slice(&ENC_H264.to_be_bytes());
            u.buf.extend_from_slice(&(unit.data.len() as u32).to_be_bytes());
            u.buf.extend_from_slice(&u32::from(first).to_be_bytes()); // reset this rect's context
            u.buf.extend_from_slice(&unit.data);
            u.rects = 1;
            st.used_h264 = true;
        } else {
            let full = !incremental || prev.len() != f.len();
            enc.changes(&mut u, &f, &prev, W, H, full, |_, _, _, _| true, &pf, encs.contains(&ENC_ZRLE))?;
        }
        st.encode_ms += t.elapsed().as_secs_f64() * 1000.0;
        if !u.finish() {
            pending = Some(incremental);
            continue;
        }
        st.frames += 1;
        if moving {
            st.moving_frames += 1;
        }
        st.bytes += u.buf.len();
        send(&u.buf)?;
        prev.clone_from(&f);
    }
    Ok(st)
}

#[test]
#[ignore]
fn h264_e2e() {
    let l = TcpListener::bind("127.0.0.1:18791").unwrap();
    let mut reports = 0;
    let mut stats = Vec::new();
    for c in l.incoming() {
        let mut c = c.unwrap();
        // Browsers can preconnect an idle HTTP socket. It must not block the next
        // viewer/report forever on this intentionally single-threaded test server.
        c.set_read_timeout(Some(Duration::from_secs(2))).unwrap();
        let mut r = BufReader::new(c.try_clone().unwrap());
        let mut line = String::new();
        if !matches!(r.read_line(&mut line), Ok(n) if n > 0) { continue; }
        let path = line.split(' ').nth(1).unwrap_or("/").to_string();
        let (mut key, mut len) = (String::new(), 0);
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
            c.set_read_timeout(None).unwrap();
            let h264 = path.contains("mode=h264");
            write!(c, "HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Accept: {}\r\n\r\n", ws::accept_key(&key)).unwrap();
            c.set_nodelay(true).unwrap();
            let st = session(c, h264).unwrap();
            println!("session {}: {st:?}", if h264 { "h264" } else { "zrle" });
            stats.push((h264, st));
        } else if path == "/report" {
            let mut body = vec![0u8; len];
            r.read_exact(&mut body).unwrap();
            let v: serde_json::Value = serde_json::from_slice(&body).unwrap();
            let samples = v["samples"].as_array().unwrap();
            // Build the expected still image once, rather than regenerating two
            // million pixels for every one of the hundreds of sampled points.
            let mut expected = vec![0; W * H];
            frame(MOVING, &mut expected, false);
            let off = |a: u32, b: u32| (0..3).map(|i| ((a >> (8 * i)) & 255).abs_diff((b >> (8 * i)) & 255)).max().unwrap();
            let worst = samples.iter().map(|s| off(s[2].as_u64().unwrap() as u32, expected[s[1].as_u64().unwrap() as usize * W + s[0].as_u64().unwrap() as usize])).max().unwrap();
            println!("report {}: webcodecs={} canvas {}x{}, {} samples, worst channel error {worst}", v["mode"], v["h264"], v["w"], v["h"], samples.len());
            assert!(worst <= 24, "picture differs from the source");
            // The viewer trace pairs each decoded frame with its own copy to the screen canvas.
            if let Some(t) = v["trace"].as_object() {
                let n = |k: &str| t[k].as_u64().unwrap_or(0);
                println!("viewer trace counts: {t:?}");
                assert!(n("draw") > 0 && n("decode") >= n("videoDraws") && n("videoDraws") >= n("draw"), "draws counted beyond decoded frames drawn");
                assert!(n("draw") <= n("screenCopies") && n("nextRaf") <= n("draw"), "draws counted without a copy to the screen");
                assert!(n("reports") >= 1, "no 5 s report");
            }
            reply(&mut c, "text/plain", b"ok");
            reports += 1;
            if reports == 2 {
                break;
            }
        } else if path.starts_with("/trace.js") {
            reply(&mut c, "text/javascript", trace_js().as_bytes());
        } else if path.starts_with("/novnc.min.js") {
            reply(&mut c, "text/javascript", NOVNC.as_bytes());
        } else {
            reply(&mut c, "text/html", PAGE.as_bytes());
        }
    }
    for (h264, st) in &stats {
        println!(
            "{:>5}: {} frames ({:.1}/s while moving), {} KB total ({} KB key frames), encode {:.2} ms/frame",
            if *h264 { "h264" } else { "zrle" },
            st.frames,
            st.moving_frames as f64 / MOVING.as_secs_f64(),
            st.bytes / 1024,
            st.key_bytes / 1024,
            st.encode_ms / st.frames.max(1) as f64
        );
    }
    assert!(stats.iter().any(|(h, s)| *h && s.used_h264), "the viewer never asked for H.264");
}
