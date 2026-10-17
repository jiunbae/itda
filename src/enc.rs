//! RFB framebuffer encoding shared by the "Share this PC" server and the RDP bridge:
//! the client's pixel format and Raw / ZRLE rectangles (64×64 tiles, solid or raw, one
//! persistent zlib stream per connection).

use flate2::{Compress, FlushCompress};
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
    pub fn put(&self, p: u32, out: &mut Vec<u8>, n: usize, skip: usize) {
        let c = [(p >> 16) & 0xff, (p >> 8) & 0xff, p & 0xff];
        let mut v = 0u32;
        for i in 0..3 {
            v |= (c[i] * self.max[i] as u32 / 255) << self.shift[i];
        }
        let b = self.bytes();
        let full = if self.big_endian { v.to_be_bytes() } else { v.to_le_bytes() };
        let full = if self.big_endian { &full[4 - b..] } else { &full[..b] };
        out.extend_from_slice(&full[skip..skip + n]);
    }
}

#[allow(clippy::too_many_arguments)]
pub fn encode_rect(out: &mut Vec<u8>, f: &[u32], fw: usize, x: usize, y: usize, w: usize, h: usize, pf: &PixelFormat, zrle: bool, z: &mut Compress) -> io::Result<()> {
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
    let mut raw = Vec::with_capacity(w * h * n + 64);
    for ty in (y..y + h).step_by(TILE) {
        let th = TILE.min(y + h - ty);
        for tx in (x..x + w).step_by(TILE) {
            let tw = TILE.min(x + w - tx);
            let first = f[ty * fw + tx];
            let solid = (ty..ty + th).all(|r| f[r * fw + tx..r * fw + tx + tw].iter().all(|&p| p == first));
            if solid {
                raw.push(1);
                pf.put(first, &mut raw, n, skip);
            } else {
                raw.push(0);
                for r in ty..ty + th {
                    for &p in &f[r * fw + tx..r * fw + tx + tw] {
                        pf.put(p, &mut raw, n, skip);
                    }
                }
            }
        }
    }
    let mut zout = Vec::with_capacity(raw.len() / 2 + 1024);
    let mut used = 0;
    loop {
        if zout.capacity() - zout.len() < 4096 {
            zout.reserve(zout.capacity().max(1 << 16));
        }
        let before = z.total_in();
        z.compress_vec(&raw[used..], &mut zout, FlushCompress::Sync).map_err(io::Error::other)?;
        used += (z.total_in() - before) as usize;
        if used >= raw.len() && zout.len() < zout.capacity() {
            break;
        }
    }
    out.extend_from_slice(&(zout.len() as u32).to_be_bytes());
    out.extend_from_slice(&zout);
    Ok(())
}

#[cfg(test)]
mod tests {
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
}
