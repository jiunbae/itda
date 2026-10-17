//! Framebuffer snapshots: box-filter downscale and a small PNG encoder (RGB, Sub filter).

use flate2::{write::ZlibEncoder, Compression, Crc};
use std::io::Write;

/// A BGRX (little-endian 0x00RRGGBB) pixel buffer.
pub struct Image {
    pub w: usize,
    pub h: usize,
    pub px: Vec<u32>,
}

impl Image {
    /// Average-of-area downscale so the result is at most `max_w` wide (never upscales).
    pub fn shrink(&self, max_w: usize) -> Image {
        if self.w <= max_w || self.w == 0 || self.h == 0 {
            return Image { w: self.w, h: self.h, px: self.px.clone() };
        }
        let (w, h) = (max_w, (self.h * max_w / self.w).max(1));
        let mut px = Vec::with_capacity(w * h);
        for y in 0..h {
            let (y0, y1) = (y * self.h / h, ((y + 1) * self.h / h).max(y * self.h / h + 1));
            for x in 0..w {
                let (x0, x1) = (x * self.w / w, ((x + 1) * self.w / w).max(x * self.w / w + 1));
                let (mut r, mut g, mut b) = (0u32, 0u32, 0u32);
                for sy in y0..y1 {
                    for &p in &self.px[sy * self.w + x0..sy * self.w + x1] {
                        r += (p >> 16) & 0xff;
                        g += (p >> 8) & 0xff;
                        b += p & 0xff;
                    }
                }
                let n = ((y1 - y0) * (x1 - x0)) as u32;
                px.push((r / n) << 16 | (g / n) << 8 | (b / n));
            }
        }
        Image { w, h, px }
    }

    pub fn png(&self, level: u32) -> Vec<u8> {
        let mut raw = Vec::with_capacity((self.w * 3 + 1) * self.h);
        for row in self.px.chunks_exact(self.w.max(1)).take(self.h) {
            raw.push(1); // Sub filter: difference to the pixel on the left
            let mut prev = [0u8; 3];
            for &p in row {
                let c = [(p >> 16) as u8, (p >> 8) as u8, p as u8];
                raw.extend_from_slice(&[c[0].wrapping_sub(prev[0]), c[1].wrapping_sub(prev[1]), c[2].wrapping_sub(prev[2])]);
                prev = c;
            }
        }
        let mut z = ZlibEncoder::new(Vec::new(), Compression::new(level));
        let _ = z.write_all(&raw);
        let idat = z.finish().unwrap_or_default();

        let mut out = b"\x89PNG\r\n\x1a\n".to_vec();
        let mut ihdr = Vec::new();
        ihdr.extend_from_slice(&(self.w as u32).to_be_bytes());
        ihdr.extend_from_slice(&(self.h as u32).to_be_bytes());
        ihdr.extend_from_slice(&[8, 2, 0, 0, 0]); // 8-bit RGB
        chunk(&mut out, b"IHDR", &ihdr);
        chunk(&mut out, b"IDAT", &idat);
        chunk(&mut out, b"IEND", &[]);
        out
    }
}

fn chunk(out: &mut Vec<u8>, kind: &[u8; 4], data: &[u8]) {
    out.extend_from_slice(&(data.len() as u32).to_be_bytes());
    out.extend_from_slice(kind);
    out.extend_from_slice(data);
    let mut crc = Crc::new();
    crc.update(kind);
    crc.update(data);
    out.extend_from_slice(&crc.sum().to_be_bytes());
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shrink_averages() {
        let img = Image { w: 4, h: 2, px: vec![0xff0000, 0x0000ff, 0, 0, 0xff0000, 0x0000ff, 0, 0] };
        let s = img.shrink(2);
        assert_eq!((s.w, s.h), (2, 1));
        assert_eq!(s.px[0], 0x7f007f);
        assert_eq!(s.px[1], 0);
    }

    #[test]
    fn png_header() {
        let p = Image { w: 3, h: 2, px: vec![0x123456; 6] }.png(6);
        assert_eq!(&p[..8], b"\x89PNG\r\n\x1a\n");
        assert_eq!(&p[12..16], b"IHDR");
    }
}
