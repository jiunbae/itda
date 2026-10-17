//! Clipboard over RFB: classic CutText (Latin-1) and the Extended Clipboard
//! pseudo-encoding (UTF-8, zlib) when the server announces it.

use crate::net::{sh, Shared, WM_CLIP};
use flate2::{read::ZlibDecoder, write::ZlibEncoder, Compression};
use std::io::{Read, Write};
use std::sync::atomic::Ordering::Relaxed;

pub const ENC_EXT_CLIPBOARD: i32 = 0xc0a1_e5ce_u32 as i32;

const TEXT: u32 = 1;
const CAPS: u32 = 1 << 24;
const REQUEST: u32 = 1 << 25;
const PEEK: u32 = 1 << 26;
const NOTIFY: u32 = 1 << 27;
const PROVIDE: u32 = 1 << 28;
const MAX_TEXT: u32 = 16 << 20;

fn ext_msg(flags: u32, data: &[u8]) -> Vec<u8> {
    let mut m = vec![6, 0, 0, 0];
    m.extend_from_slice(&(-(4 + data.len() as i32)).to_be_bytes());
    m.extend_from_slice(&flags.to_be_bytes());
    m.extend_from_slice(data);
    m
}

/// UI side: the local clipboard changed (text uses \n line endings on the wire).
pub fn send_local(text: &str) {
    let s = sh();
    let t = text.replace("\r\n", "\n");
    *s.clip_out.lock().unwrap() = t.clone();
    if s.ext_clip.load(Relaxed) {
        s.send(&ext_msg(NOTIFY | TEXT, &[]));
        return;
    }
    // Classic CutText is Latin-1; for anything beyond it UTF-8 is the best guess.
    let bytes: Vec<u8> = if t.chars().all(|c| (c as u32) < 0x100) { t.chars().map(|c| c as u8).collect() } else { t.into_bytes() };
    let mut m = vec![6, 0, 0, 0];
    m.extend_from_slice(&(bytes.len() as u32).to_be_bytes());
    m.extend_from_slice(&bytes);
    s.send(&m);
}

fn deliver(s: &Shared, text: String) {
    *s.clip_in.lock().unwrap() = Some(text);
    s.post(WM_CLIP);
}

/// Classic ServerCutText payload.
pub fn on_classic(s: &Shared, bytes: Vec<u8>) {
    crate::log!("ServerCutText (classic) {} bytes", bytes.len());
    let text = String::from_utf8(bytes).unwrap_or_else(|e| e.into_bytes().iter().map(|&b| b as char).collect());
    deliver(s, text);
}

/// Extended clipboard message (ServerCutText with negative length).
pub fn on_extended(s: &Shared, p: &[u8]) {
    if p.len() < 4 {
        return;
    }
    let flags = u32::from_be_bytes([p[0], p[1], p[2], p[3]]);
    let data = &p[4..];
    if flags & CAPS != 0 {
        crate::log!("extended clipboard supported (server caps {flags:#010x})");
        s.ext_clip.store(true, Relaxed);
        s.send(&ext_msg(CAPS | TEXT | REQUEST | PEEK | NOTIFY | PROVIDE, &MAX_TEXT.to_be_bytes()));
    } else if flags & REQUEST != 0 && flags & TEXT != 0 {
        let t = s.clip_out.lock().unwrap().clone();
        let mut inner = ((t.len() + 1) as u32).to_be_bytes().to_vec();
        inner.extend_from_slice(t.as_bytes());
        inner.push(0);
        let mut z = ZlibEncoder::new(Vec::new(), Compression::fast());
        let _ = z.write_all(&inner);
        s.send(&ext_msg(PROVIDE | TEXT, &z.finish().unwrap_or_default()));
    } else if flags & PEEK != 0 {
        s.send(&ext_msg(NOTIFY | TEXT, &[])); // which formats we have
    } else if flags & NOTIFY != 0 {
        if flags & TEXT != 0 {
            s.send(&ext_msg(REQUEST | TEXT, &[])); // server has new text: fetch it
        }
    } else if flags & PROVIDE != 0 && flags & TEXT != 0 {
        let mut inner = Vec::new();
        let _ = ZlibDecoder::new(data).read_to_end(&mut inner);
        if inner.len() < 4 {
            return;
        }
        let n = u32::from_be_bytes([inner[0], inner[1], inner[2], inner[3]]) as usize;
        let body = &inner[4..(4 + n).min(inner.len())];
        let body = body.strip_suffix(&[0]).unwrap_or(body);
        crate::log!("clipboard from server: {} bytes (UTF-8)", body.len());
        deliver(s, String::from_utf8_lossy(body).into_owned());
    }
}
