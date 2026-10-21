//! RFB framebuffer encoding shared by the "Share this PC" server and the RDP bridge:
//! the client's pixel format and Raw / ZRLE rectangles (64×64 tiles, solid, packed palette
//! or raw, one persistent zlib stream per connection).

use flate2::{Compress, Compression, FlushCompress};
use std::io;

pub const TILE: usize = 64;
pub const ENC_RAW: i32 = 0;
pub const ENC_ZRLE: i32 = 16;

#[derive(Clone, Copy, PartialEq, Debug)]
pub struct PixelFormat {
    bpp: u8,
    big_endian: bool,
    max: [u16; 3],
    shift: [u8; 3],
}

impl PixelFormat {
    pub fn parse(b: &[u8]) -> PixelFormat {
        let u16_at = |i: usize| u16::from_be_bytes([b[i], b[i + 1]]);
        PixelFormat { bpp: b[0], big_endian: b[2] != 0, max: [u16_at(4), u16_at(6), u16_at(8)], shift: [b[10], b[11], b[12]] }
    }

    /// A viewer's SetPixelFormat: only what this encoder can write (true colour in 8, 16 or 32
    /// bits, each channel inside the pixel); anything else ends the connection.
    pub fn from_viewer(b: &[u8]) -> std::io::Result<PixelFormat> {
        let pf = PixelFormat::parse(b);
        let fits = pf.shift.iter().zip(pf.max).all(|(&s, m)| m > 0 && (s as u32) + (32 - (m as u32).leading_zeros()) <= pf.bpp as u32);
        if ![8, 16, 32].contains(&pf.bpp) || b[3] == 0 || !fits {
            return Err(std::io::Error::new(std::io::ErrorKind::InvalidData, "unsupported pixel format"));
        }
        Ok(pf)
    }

    pub fn bytes(&self) -> usize {
        (self.bpp as usize / 8).max(1)
    }

    /// ZRLE's compact pixel: 3 bytes when a 32-bit pixel's colour fits in its low or high 3 bytes.
    pub fn cpixel(&self) -> (usize, usize) {
        if self.bpp == 32 && self.max.iter().all(|&m| m <= 255) {
            let top = self.shift.iter().zip(self.max).map(|(&s, m)| s as u32 + (32 - (m as u32).leading_zeros())).max().unwrap_or(32);
            let low = *self.shift.iter().min().unwrap_or(&0);
            if top <= 24 {
                return (3, if self.big_endian { 1 } else { 0 });
            }
            if low >= 8 {
                return (3, if self.big_endian { 0 } else { 1 });
            }
        }
        (self.bytes(), 0)
    }

    /// Append pixel `p` (0x00RRGGBB) as `n` bytes starting at byte `skip` of the full pixel.
    #[inline]
    pub fn put(&self, p: u32, out: &mut Vec<u8>, n: usize, skip: usize) {
        let c = [(p >> 16) & 0xff, (p >> 8) & 0xff, p & 0xff];
        // 8 bits a channel (every true-colour viewer) needs no rescaling.
        let v = if self.max == [255; 3] {
            c[0] << self.shift[0] | c[1] << self.shift[1] | c[2] << self.shift[2]
        } else {
            (0..3).fold(0, |v, i| v | (c[i] * self.max[i] as u32 / 255) << self.shift[i])
        };
        let b = self.bytes();
        let full = if self.big_endian { v.to_be_bytes() } else { v.to_le_bytes() };
        let full = if self.big_endian { &full[4 - b..] } else { &full[..b] };
        out.extend_from_slice(&full[skip..skip + n]);
    }
}

/// One connection's ZRLE state: the persistent zlib stream and scratch buffers reused for
/// every rectangle.
pub struct Encoder {
    z: Compress,
    raw: Vec<u8>,
    zout: Vec<u8>,
    /// Packed-palette tiles (2–16 colours); off only to compare in the bench.
    palette: bool,
}

/// zlib level of the ZRLE stream.
pub const LEVEL: u32 = 5;

impl Encoder {
    pub fn new() -> Encoder {
        Encoder::with(LEVEL, true)
    }

    fn with(level: u32, palette: bool) -> Encoder {
        Encoder { z: Compress::new(Compression::new(level), true), raw: Vec::new(), zout: Vec::new(), palette }
    }

    #[allow(clippy::too_many_arguments)]
    pub fn rect(&mut self, out: &mut Vec<u8>, f: &[u32], fw: usize, x: usize, y: usize, w: usize, h: usize, pf: &PixelFormat, zrle: bool) -> io::Result<()> {
        out.extend_from_slice(&(x as u16).to_be_bytes());
        out.extend_from_slice(&(y as u16).to_be_bytes());
        out.extend_from_slice(&(w as u16).to_be_bytes());
        out.extend_from_slice(&(h as u16).to_be_bytes());
        if !zrle {
            out.extend_from_slice(&ENC_RAW.to_be_bytes());
            let b = pf.bytes();
            for row in y..y + h {
                for &p in &f[row * fw + x..row * fw + x + w] {
                    pf.put(p, out, b, 0);
                }
            }
            return Ok(());
        }
        out.extend_from_slice(&ENC_ZRLE.to_be_bytes());
        let (n, skip) = pf.cpixel();
        let raw = &mut self.raw;
        raw.clear();
        let mut pal = [0u32; 16];
        for ty in (y..y + h).step_by(TILE) {
            let th = TILE.min(y + h - ty);
            for tx in (x..x + w).step_by(TILE) {
                let tw = TILE.min(x + w - tx);
                let rows = || (ty..ty + th).map(|r| &f[r * fw + tx..r * fw + tx + tw]);
                let k = if self.palette { colours(rows(), &mut pal) } else { solid(rows(), &mut pal) };
                let bits = match k { 2 => 1, 3..=4 => 2, _ => 4 };
                if k == 1 {
                    raw.push(1);
                    pf.put(pal[0], raw, n, skip);
                } else if k <= 16 && k * n + (tw * bits).div_ceil(8) * th < tw * th * n {
                    raw.push(k as u8);
                    for &c in &pal[..k] {
                        pf.put(c, raw, n, skip);
                    }
                    let mut last = 0;
                    for row in rows() {
                        // Indices MSB first, each row padded to a whole byte.
                        let (mut acc, mut used) = (0u8, 0);
                        for &p in row {
                            if pal[last] != p {
                                last = pal[..k].iter().position(|&c| c == p).expect("tile pixel is in its palette");
                            }
                            acc |= (last as u8) << (8 - bits - used);
                            used += bits;
                            if used == 8 {
                                raw.push(acc);
                                (acc, used) = (0, 0);
                            }
                        }
                        if used > 0 {
                            raw.push(acc);
                        }
                    }
                } else {
                    raw.push(0);
                    for row in rows() {
                        for &p in row {
                            pf.put(p, raw, n, skip);
                        }
                    }
                }
            }
        }
        let zout = &mut self.zout;
        zout.clear();
        let mut used = 0;
        loop {
            if zout.capacity() - zout.len() < 4096 {
                zout.reserve(zout.capacity().max(1 << 16));
            }
            let before = self.z.total_in();
            self.z.compress_vec(&raw[used..], zout, FlushCompress::Sync).map_err(io::Error::other)?;
            used += (self.z.total_in() - before) as usize;
            if used >= raw.len() && zout.len() < zout.capacity() {
                break;
            }
        }
        out.extend_from_slice(&(zout.len() as u32).to_be_bytes());
        out.extend_from_slice(zout);
        Ok(())
    }
}

impl Encoder {
    /// Encode into `u` the tiles of `frame` (w×h) that `want(x, y, w, h)` admits and that
    /// differ from `prev` (all of them when `full`), merged into horizontal runs per tile row.
    #[allow(clippy::too_many_arguments)]
    pub fn changes(&mut self, u: &mut Update, frame: &[u32], prev: &[u32], w: usize, h: usize, full: bool, want: impl Fn(usize, usize, usize, usize) -> bool, pf: &PixelFormat, zrle: bool) -> io::Result<()> {
        for ty in (0..h).step_by(TILE) {
            let th = TILE.min(h - ty);
            let mut run: Option<usize> = None;
            for tx in (0..w).step_by(TILE).chain([w]) {
                let dirty = tx < w && {
                    let tw = TILE.min(w - tx);
                    want(tx, ty, tw, th) && (full || (ty..ty + th).any(|y| frame[y * w + tx..y * w + tx + tw] != prev[y * w + tx..y * w + tx + tw]))
                };
                match (dirty, run) {
                    (true, None) => run = Some(tx),
                    (false, Some(start)) => {
                        self.rect(&mut u.buf, frame, w, start, ty, tx - start, th, pf, zrle)?;
                        u.rects += 1;
                        run = None;
                    }
                    _ => {}
                }
            }
        }
        Ok(())
    }
}

/// A FramebufferUpdate built in one reused buffer: the header goes first, its rectangle
/// count is filled in at the end.
#[derive(Default)]
pub struct Update {
    pub buf: Vec<u8>,
    pub rects: u16,
}

impl Update {
    pub fn start(&mut self) {
        self.buf.clear();
        self.buf.extend_from_slice(&[0, 0, 0, 0]);
        self.rects = 0;
    }

    /// Fill in the rectangle count; false when there is nothing to send.
    pub fn finish(&mut self) -> bool {
        self.buf[2..4].copy_from_slice(&self.rects.to_be_bytes());
        self.rects > 0
    }
}

pub const ENC_EXT_DESKTOP_SIZE: i32 = -308;

/// One display inside the framebuffer, as ExtendedDesktopSize describes it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Screen {
    pub id: u32,
    pub x: u16,
    pub y: u16,
    pub w: u16,
    pub h: u16,
}

/// An ExtendedDesktopSize rectangle: `reason` 0 = the server changed, 1 = answer to this
/// client's SetDesktopSize (with `status`: 0 done, 1 not allowed, …), the framebuffer size
/// and its displays.
/// "Open H.264" (RFB encoding 50): a rectangle of Annex B H.264, as noVNC decodes it.
pub const ENC_H264: i32 = 50;
/// Reset the decoder context of this rectangle before decoding (flags of an H.264 rectangle).
pub const H264_RESET: u32 = 1;

impl Update {
    /// An H.264 rectangle: `data` is one or more Annex B access units for the context (x, y, w, h).
    pub fn h264(&mut self, x: u16, y: u16, w: u16, h: u16, flags: u32, data: &[u8]) {
        for v in [x, y, w, h] {
            self.buf.extend_from_slice(&v.to_be_bytes());
        }
        self.buf.extend_from_slice(&ENC_H264.to_be_bytes());
        self.buf.extend_from_slice(&(data.len() as u32).to_be_bytes());
        self.buf.extend_from_slice(&flags.to_be_bytes());
        self.buf.extend_from_slice(data);
        self.rects += 1;
    }
}

pub fn ext_desktop_size(out: &mut Vec<u8>, reason: u16, status: u16, w: u16, h: u16, screens: &[Screen]) {
    out.extend_from_slice(&reason.to_be_bytes());
    out.extend_from_slice(&status.to_be_bytes());
    out.extend_from_slice(&w.to_be_bytes());
    out.extend_from_slice(&h.to_be_bytes());
    out.extend_from_slice(&ENC_EXT_DESKTOP_SIZE.to_be_bytes());
    out.extend_from_slice(&[screens.len().min(255) as u8, 0, 0, 0]);
    for s in screens.iter().take(255) {
        out.extend_from_slice(&s.id.to_be_bytes());
        for v in [s.x, s.y, s.w, s.h] {
            out.extend_from_slice(&v.to_be_bytes());
        }
        out.extend_from_slice(&0u32.to_be_bytes()); // flags
    }
}

/// Displays given in desktop coordinates (id, left, top, right, bottom) placed in a w×h
/// framebuffer whose top-left is the desktop point (ox, oy): clipped to it, empty ones
/// dropped, ordered left to right, then top to bottom. None of them: one display, all of it.
pub fn screens_in(ox: i32, oy: i32, w: usize, h: usize, monitors: &[(u32, i32, i32, i32, i32)]) -> Vec<Screen> {
    let (w, h) = (w.min(u16::MAX as usize) as i32, h.min(u16::MAX as usize) as i32);
    let mut out: Vec<Screen> = monitors
        .iter()
        .filter_map(|&(id, l, t, r, b)| {
            let (x0, y0) = ((l - ox).clamp(0, w), (t - oy).clamp(0, h));
            let (x1, y1) = ((r - ox).clamp(0, w), (b - oy).clamp(0, h));
            (x1 > x0 && y1 > y0).then(|| Screen { id, x: x0 as u16, y: y0 as u16, w: (x1 - x0) as u16, h: (y1 - y0) as u16 })
        })
        .collect();
    out.sort_by_key(|s| (s.x, s.y));
    if out.is_empty() {
        out.push(Screen { id: 0, x: 0, y: 0, w: w as u16, h: h as u16 });
    }
    out
}

/// The parts of a framebuffer changed since the last update: rectangles (x, y, w, h), or all
/// of it. Past MAX_DAMAGE rectangles they merge into their bounding box.
#[derive(Default)]
pub struct Damage {
    rects: Vec<(usize, usize, usize, usize)>,
    all: bool,
}

const MAX_DAMAGE: usize = 32;

impl Damage {
    pub fn add(&mut self, x: usize, y: usize, w: usize, h: usize) {
        if self.all || w == 0 || h == 0 {
            return;
        }
        if self.rects.len() >= MAX_DAMAGE {
            let (mut x0, mut y0, mut x1, mut y1) = (x, y, x + w, y + h);
            for &(rx, ry, rw, rh) in &self.rects {
                (x0, y0, x1, y1) = (x0.min(rx), y0.min(ry), x1.max(rx + rw), y1.max(ry + rh));
            }
            self.rects.clear();
            self.rects.push((x0, y0, x1 - x0, y1 - y0));
            return;
        }
        self.rects.push((x, y, w, h));
    }

    pub fn everything(&mut self) {
        self.all = true;
        self.rects.clear();
    }

    pub fn clear(&mut self) {
        self.all = false;
        self.rects.clear();
    }

    pub fn is_empty(&self) -> bool {
        !self.all && self.rects.is_empty()
    }

    pub fn is_all(&self) -> bool {
        self.all
    }

    pub fn rects(&self) -> &[(usize, usize, usize, usize)] {
        &self.rects
    }

    pub fn hits(&self, x: usize, y: usize, w: usize, h: usize) -> bool {
        self.all || self.rects.iter().any(|&(rx, ry, rw, rh)| x < rx + rw && rx < x + w && y < ry + rh && ry < y + h)
    }
}

/// The tile's distinct colours into `pal`: their count, or 17 once there are more than 16.
fn colours<'a>(rows: impl Iterator<Item = &'a [u32]>, pal: &mut [u32; 16]) -> usize {
    let mut k = 0;
    let mut last = 0;
    for row in rows {
        for &p in row {
            if k > 0 && pal[last] == p {
                continue;
            }
            match pal[..k].iter().position(|&c| c == p) {
                Some(i) => last = i,
                None if k == 16 => return 17,
                None => {
                    pal[k] = p;
                    last = k;
                    k += 1;
                }
            }
        }
    }
    k
}

/// 1 when the tile is one colour (`pal[0]`), otherwise 17.
fn solid<'a>(rows: impl Iterator<Item = &'a [u32]>, pal: &mut [u32; 16]) -> usize {
    let mut rows = rows.peekable();
    pal[0] = rows.peek().map_or(0, |r| r[0]);
    if rows.all(|r| r.iter().all(|&p| p == pal[0])) {
        1
    } else {
        17
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn viewer_pixel_formats_are_checked() {
        // bpp, depth, big endian, true colour, max r/g/b, shift r/g/b
        let pf = |bpp: u8, tc: u8, max: u16, shift: [u8; 3]| {
            let m = max.to_be_bytes();
            [bpp, 24, 0, tc, m[0], m[1], m[0], m[1], m[0], m[1], shift[0], shift[1], shift[2], 0, 0, 0]
        };
        assert!(PixelFormat::from_viewer(&pf(32, 1, 255, [16, 8, 0])).is_ok());
        assert!(PixelFormat::from_viewer(&pf(16, 1, 31, [10, 5, 0])).is_ok());
        assert!(PixelFormat::from_viewer(&pf(8, 1, 3, [4, 2, 0])).is_ok());
        for bad in [pf(248, 1, 255, [16, 8, 0]), pf(24, 1, 255, [16, 8, 0]), pf(32, 0, 255, [16, 8, 0]), pf(32, 1, 255, [200, 8, 0]), pf(16, 1, 255, [10, 5, 0]), pf(32, 1, 0, [16, 8, 0])] {
            assert!(PixelFormat::from_viewer(&bad).is_err(), "{bad:?}");
        }
    }

    use super::*;
    use crate::rfb::PIXEL_FORMAT;

    #[test]
    fn pixel_formats() {
        let ours = PixelFormat::parse(&PIXEL_FORMAT);
        assert_eq!(ours.cpixel(), (3, 0));
        let mut v = Vec::new();
        ours.put(0x123456, &mut v, 3, 0);
        assert_eq!(v, [0x56, 0x34, 0x12]); // B G R
        // noVNC: red at shift 0
        let novnc = PixelFormat { bpp: 32, big_endian: false, max: [255; 3], shift: [0, 8, 16] };
        v.clear();
        novnc.put(0x123456, &mut v, 3, 0);
        assert_eq!(v, [0x12, 0x34, 0x56]); // R G B
        // 16-bit 565
        let rgb565 = PixelFormat { bpp: 16, big_endian: false, max: [31, 63, 31], shift: [11, 5, 0] };
        assert_eq!(rgb565.cpixel(), (2, 0));
        v.clear();
        rgb565.put(0xffffff, &mut v, 2, 0);
        assert_eq!(v, [0xff, 0xff]);
    }

    /// An independent ZRLE reader: each pixel as the CPIXEL bytes it decodes to.
    fn decode(msg: &[u8], z: &mut flate2::Decompress, cp: usize, seen: &mut [bool; 256]) -> (usize, usize, usize, usize, Vec<Vec<u8>>) {
        let u16_at = |i: usize| u16::from_be_bytes([msg[i], msg[i + 1]]) as usize;
        let (x, y, w, h) = (u16_at(0), u16_at(2), u16_at(4), u16_at(6));
        assert_eq!(i32::from_be_bytes(msg[8..12].try_into().unwrap()), ENC_ZRLE);
        let len = u32::from_be_bytes(msg[12..16].try_into().unwrap()) as usize;
        assert_eq!(msg.len(), 16 + len, "one rectangle, nothing after it");
        let mut d = Vec::with_capacity(w * h * cp * 2 + 1024);
        z.decompress_vec(&msg[16..], &mut d, flate2::FlushDecompress::Sync).unwrap();
        let mut px = vec![Vec::new(); w * h];
        let mut i = 0;
        let mut take = |n: usize| {
            i += n;
            d[i - n..i].to_vec()
        };
        for ty in (0..h).step_by(TILE) {
            let th = TILE.min(h - ty);
            for tx in (0..w).step_by(TILE) {
                let tw = TILE.min(w - tx);
                let sub = take(1)[0] as usize;
                seen[sub] = true;
                let at = |j: usize| (ty + j / tw) * w + tx + j % tw;
                match sub {
                    0 => (0..tw * th).for_each(|j| px[at(j)] = take(cp)),
                    1 => {
                        let c = take(cp);
                        (0..tw * th).for_each(|j| px[at(j)] = c.clone());
                    }
                    2..=16 => {
                        let pal: Vec<_> = (0..sub).map(|_| take(cp)).collect();
                        let bits = match sub { 2 => 1, 3..=4 => 2, _ => 4 };
                        for r in 0..th {
                            let row = take((tw * bits).div_ceil(8));
                            for c in 0..tw {
                                let v = (row[c * bits / 8] >> (8 - bits - c * bits % 8)) as usize & ((1 << bits) - 1);
                                px[at(r * tw + c)] = pal[v].clone();
                            }
                        }
                    }
                    s => panic!("unexpected subencoding {s}"),
                }
            }
        }
        assert_eq!(i, d.len(), "all inflated bytes used");
        (x, y, w, h, px)
    }

    /// Tiles with 1..17 colours, partial tiles, several rectangles over one stream, in every
    /// pixel format a viewer may ask for.
    #[test]
    fn zrle_round_trip() {
        let formats = [
            PixelFormat::parse(&PIXEL_FORMAT),
            PixelFormat { bpp: 32, big_endian: false, max: [255; 3], shift: [0, 8, 16] },
            PixelFormat { bpp: 32, big_endian: true, max: [255; 3], shift: [16, 8, 0] },
            PixelFormat { bpp: 32, big_endian: false, max: [255; 3], shift: [24, 16, 8] },
            PixelFormat { bpp: 16, big_endian: false, max: [31, 63, 31], shift: [11, 5, 0] },
            PixelFormat { bpp: 8, big_endian: false, max: [7, 7, 3], shift: [5, 2, 0] },
        ];
        let (fw, fh) = (300, 200);
        let mut f = vec![0u32; fw * fh];
        let mut seed = 1u64;
        // Tile t holds t+1 colours (1..=17), noise after that.
        for (i, p) in f.iter_mut().enumerate() {
            seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            let t = (i % fw / TILE + i / fw / TILE * fw.div_ceil(TILE)) as u32;
            *p = if t < 17 { ((seed >> 40) as u32 % (t + 1)) * 0x0f0b07 } else { (seed >> 32) as u32 & 0xffffff };
        }
        for pf in formats {
            let mut e = Encoder::new();
            let mut z = flate2::Decompress::new(true);
            let mut seen = [false; 256];
            for (x, y, w, h) in [(0, 0, fw, fh), (5, 3, 70, 65), (0, 0, 1, 1), (17, 130, 283, 70), (0, 0, fw, fh)] {
                let mut out = Vec::new();
                e.rect(&mut out, &f, fw, x, y, w, h, &pf, true).unwrap();
                check(&pf, &f, fw, (x, y, w, h), &out, &mut z, &mut seen);
            }
            for sub in [0, 1, 2, 3, 4, 5, 16] {
                assert!(seen[sub], "{pf:?}: no tile used subencoding {sub}");
            }
        }
    }

    fn check(pf: &PixelFormat, f: &[u32], fw: usize, (x, y, w, h): (usize, usize, usize, usize), msg: &[u8], z: &mut flate2::Decompress, seen: &mut [bool; 256]) {
        let (cp, skip) = pf.cpixel();
        let (dx, dy, dw, dh, px) = decode(msg, z, cp, seen);
        assert_eq!((dx, dy, dw, dh), (x, y, w, h));
        for r in 0..h {
            for c in 0..w {
                let mut want = Vec::new();
                pf.put(f[(y + r) * fw + x + c], &mut want, cp, skip);
                assert_eq!(px[r * w + c], want, "{pf:?} rect {x},{y} {w}x{h} pixel {c},{r}");
            }
        }
    }

    /// A viewer may switch pixel format and mix Raw rectangles into one ZRLE stream.
    #[test]
    fn zrle_format_switch_and_raw() {
        let fw = 130;
        let f: Vec<u32> = (0..fw * 70).map(|i| [0x102030, 0xffffff, 0x4744b0][i % 7 % 3]).collect();
        let a = PixelFormat::parse(&PIXEL_FORMAT);
        let b = PixelFormat { bpp: 16, big_endian: false, max: [31, 63, 31], shift: [11, 5, 0] };
        let mut e = Encoder::new();
        let mut z = flate2::Decompress::new(true);
        let mut seen = [false; 256];
        for (pf, zrle) in [(a, true), (b, true), (a, false), (a, true), (b, false), (b, true)] {
            let r = (3, 2, 100, 66);
            let mut out = Vec::new();
            e.rect(&mut out, &f, fw, r.0, r.1, r.2, r.3, &pf, zrle).unwrap();
            if zrle {
                check(&pf, &f, fw, r, &out, &mut z, &mut seen);
            } else {
                assert_eq!(out.len(), 12 + r.2 * r.3 * pf.bytes());
            }
        }
    }

    #[test]
    fn ext_desktop_size_bytes() {
        let mut out = Vec::new();
        ext_desktop_size(&mut out, 1, 1, 3840, 1080, &[Screen { id: 7, x: 0, y: 0, w: 1920, h: 1080 }, Screen { id: 9, x: 1920, y: 0, w: 1920, h: 1080 }]);
        assert_eq!(&out[..12], &[0, 1, 0, 1, 0x0f, 0, 0x04, 0x38, 0xff, 0xff, 0xfe, 0xcc]);
        assert_eq!(&out[12..16], &[2, 0, 0, 0]);
        assert_eq!(&out[16..32], &[0, 0, 0, 7, 0, 0, 0, 0, 0x07, 0x80, 0x04, 0x38, 0, 0, 0, 0]);
        assert_eq!(&out[32..36], &[0, 0, 0, 9]);
        assert_eq!(&out[36..38], &[0x07, 0x80]); // x = 1920
        assert_eq!(out.len(), 16 + 2 * 16);
    }

    /// Windows desktop coordinates (the primary display at 0,0, others negative or beyond)
    /// placed in a framebuffer that starts at the virtual desktop's corner.
    #[test]
    fn displays_in_the_framebuffer() {
        // A second display to the left of the primary, lower down: virtual origin (-1280, 0).
        let s = screens_in(-1280, 0, 3200, 1200, &[(1, 0, 0, 1920, 1080), (2, -1280, 200, 0, 1200)]);
        assert_eq!(s, [Screen { id: 2, x: 0, y: 200, w: 1280, h: 1000 }, Screen { id: 1, x: 1280, y: 0, w: 1920, h: 1080 }]);
        // Stacked: ordered left to right, then top to bottom.
        let s = screens_in(0, -1080, 1920, 2160, &[(1, 0, 0, 1920, 1080), (2, 0, -1080, 1920, 0)]);
        assert_eq!(s.iter().map(|s| (s.id, s.y)).collect::<Vec<_>>(), [(2, 0), (1, 1080)]);
        // Clipped to the framebuffer; nothing known -> one display covering it.
        assert_eq!(screens_in(0, 0, 100, 100, &[(1, -50, -50, 50, 50)]), [Screen { id: 1, x: 0, y: 0, w: 50, h: 50 }]);
        assert_eq!(screens_in(0, 0, 800, 600, &[]), [Screen { id: 0, x: 0, y: 0, w: 800, h: 600 }]);
    }

    #[test]
    fn damage() {
        let mut d = Damage::default();
        assert!(d.is_empty() && !d.hits(0, 0, 64, 64));
        d.add(100, 10, 5, 5);
        d.add(0, 0, 0, 9); // empty: ignored
        assert!(d.hits(64, 0, 64, 64) && !d.hits(0, 0, 64, 64) && !d.hits(128, 0, 64, 64));
        for i in 0..MAX_DAMAGE {
            d.add(i * 10, 300, 1, 1);
        }
        assert_eq!(d.rects(), [(0, 10, 311, 291)], "merged into the bounding box");
        d.everything();
        assert!(d.is_all() && d.hits(9999, 9999, 1, 1));
        d.clear();
        assert!(d.is_empty());
    }

    /// Only changed tiles go out, neighbours merged into one rectangle per tile row.
    #[test]
    fn changed_runs() {
        let (w, h) = (300, 130);
        let prev = vec![0x101010u32; w * h];
        let mut f = prev.clone();
        for x in [5, 70, 140, 299] {
            f[3 * w + x] = 0xffffff; // tiles 0, 1, 2 and 4 of row 0
        }
        f[129 * w] = 0xffffff; // tile 0 of row 2
        let pf = PixelFormat::parse(&PIXEL_FORMAT);
        let mut e = Encoder::new();
        let mut u = Update::default();
        let rects = |u: &Update| {
            let mut v = Vec::new();
            let mut i = 4;
            while i < u.buf.len() {
                let at = |j: usize| u16::from_be_bytes([u.buf[i + j], u.buf[i + j + 1]]) as usize;
                v.push((at(0), at(2), at(4), at(6)));
                i += 16 + u32::from_be_bytes(u.buf[i + 12..i + 16].try_into().unwrap()) as usize;
            }
            v
        };
        u.start();
        e.changes(&mut u, &f, &prev, w, h, false, |_, _, _, _| true, &pf, true).unwrap();
        assert!(u.finish());
        assert_eq!(u16::from_be_bytes([u.buf[2], u.buf[3]]), 3);
        assert_eq!(rects(&u), [(0, 0, 192, 64), (256, 0, 44, 64), (0, 128, 64, 2)]);
        // `want` can leave tiles out; `full` sends every admitted one.
        u.start();
        e.changes(&mut u, &f, &prev, w, h, true, |x, y, _, _| x < 128 && y < 64, &pf, true).unwrap();
        assert_eq!(rects(&u), [(0, 0, 128, 64)]);
        u.start();
        e.changes(&mut u, &prev, &prev, w, h, false, |_, _, _, _| true, &pf, true).unwrap();
        assert!(!u.finish(), "nothing changed");
    }

    /// A palette that costs more than the raw pixels is not used (8-bit pixels, 2×1 tile).
    #[test]
    fn zrle_tiny_tile_stays_raw() {
        let pf = PixelFormat { bpp: 8, big_endian: false, max: [7, 7, 3], shift: [5, 2, 0] };
        let f = [0x000000, 0xffffff];
        let mut out = Vec::new();
        Encoder::new().rect(&mut out, &f, 2, 0, 0, 2, 1, &pf, true).unwrap();
        let mut seen = [false; 256];
        check(&pf, &f, 2, (0, 0, 2, 1), &out, &mut flate2::Decompress::new(true), &mut seen);
        assert!(seen[0] && !seen[2]);
    }

    /// The 8-bit fast path agrees with the general scaling.
    #[test]
    fn put_fast_path() {
        let pf = PixelFormat { bpp: 32, big_endian: false, max: [255; 3], shift: [0, 8, 16] };
        for p in [0, 0xffffff, 0x123456, 0x80ff01] {
            let c = [(p >> 16) & 0xff, (p >> 8) & 0xff, p & 0xff];
            let slow = (0..3).fold(0u32, |v, i| v | (c[i] * 255 / 255) << pf.shift[i]);
            let mut v = Vec::new();
            pf.put(p, &mut v, 4, 0);
            assert_eq!(v, slow.to_le_bytes());
        }
    }
}

/// Synthetic desktop content for the encoder bench and tests.
#[cfg(test)]
pub(crate) mod frames {
    pub const W: usize = 1920;
    pub const H: usize = 1080;

    fn rng(seed: &mut u64) -> u32 {
        *seed ^= *seed << 13;
        *seed ^= *seed >> 7;
        *seed ^= *seed << 17;
        (*seed >> 16) as u32
    }

    /// Dark text on white with 4 anti-aliasing greys, in lines like a document or terminal.
    pub fn text() -> Vec<u32> {
        let mut f = vec![0xffffff; W * H];
        let mut s = 7;
        let shades = [0x222222, 0x666666, 0x999999, 0xcccccc];
        for line in (20..H - 20).step_by(22) {
            let mut x = 40;
            while x < W - 60 {
                let word = 3 + rng(&mut s) as usize % 9;
                for _ in 0..word * 9 {
                    for y in line..line + 14 {
                        if rng(&mut s) % 3 == 0 {
                            f[y * W + x] = shades[rng(&mut s) as usize % 4];
                        }
                    }
                    x += 1;
                }
                x += 9;
            }
        }
        f
    }

    /// Flat panels, borders, buttons and a few accent colours, like an app window.
    pub fn ui() -> Vec<u32> {
        let mut f = vec![0xf3f3f3; W * H];
        let mut s = 11;
        let fills = [0xffffff, 0xe1e1e1, 0x4744b0, 0x33d1c2, 0x202020];
        for _ in 0..400 {
            let (x, y) = (rng(&mut s) as usize % (W - 200), rng(&mut s) as usize % (H - 60));
            let (w, h) = (20 + rng(&mut s) as usize % 180, 10 + rng(&mut s) as usize % 50);
            let c = fills[rng(&mut s) as usize % fills.len()];
            for r in y..y + h {
                for p in &mut f[r * W + x..r * W + x + w] {
                    *p = c;
                }
                f[r * W + x] = 0xc8c8c8;
                f[r * W + x + w - 1] = 0xc8c8c8;
            }
        }
        f
    }

    /// A smooth gradient (wallpaper).
    pub fn gradient() -> Vec<u32> {
        (0..W * H).map(|i| ((i % W * 255 / W) as u32) << 16 | ((i / W * 255 / H) as u32) << 8 | 0x80).collect()
    }

    /// Photo or video: every pixel different.
    pub fn noise() -> Vec<u32> {
        let mut s = 3;
        (0..W * H).map(|_| rng(&mut s) & 0xffffff).collect()
    }
}

#[cfg(test)]
mod bench {
    use super::frames::*;
    use super::*;
    use crate::rfb::PIXEL_FORMAT;
    use std::time::Instant;

    /// `cargo test --release enc::bench -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn zrle() {
        let pf = PixelFormat::parse(&PIXEL_FORMAT);
        for (name, f) in [("text", text()), ("ui", ui()), ("gradient", gradient()), ("noise", noise())] {
            for (level, palette) in [(5, false), (1, true), (3, true), (5, true)] {
                let mut e = Encoder::with(level, palette);
                let mut out = Vec::new();
                let t = Instant::now();
                const N: u32 = 5;
                for _ in 0..N {
                    out.clear();
                    e.rect(&mut out, &f, W, 0, 0, W, H, &pf, true).unwrap();
                }
                let ms = t.elapsed().as_secs_f64() * 1000.0 / N as f64;
                println!("{name:>8} level {level} palette {palette:>5}: {:>8} B  {ms:>6.1} ms/frame", out.len());
            }
        }
    }
}
