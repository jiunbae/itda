//! Framebuffer backed by a GDI DIB section, plus the pixel decoders that write into it.

use std::ptr::null_mut;
use windows_sys::Win32::Graphics::Gdi::*;

pub struct Fb {
    pub w: usize,
    pub h: usize,
    pub hbm: HBITMAP,
    bits: *mut u32,
    /// Union of changed pixels since the UI last looked: [x0, y0, x1, y1].
    pub dirty: Option<[i32; 4]>,
}

// The DIB memory is only touched while holding the Fb mutex.
unsafe impl Send for Fb {}

pub fn dib_info(w: i32, h: i32) -> BITMAPINFO {
    let mut bi: BITMAPINFO = unsafe { std::mem::zeroed() };
    bi.bmiHeader.biSize = size_of::<BITMAPINFOHEADER>() as u32;
    bi.bmiHeader.biWidth = w;
    bi.bmiHeader.biHeight = -h; // top-down
    bi.bmiHeader.biPlanes = 1;
    bi.bmiHeader.biBitCount = 32;
    bi
}

impl Fb {
    pub fn new(w: usize, h: usize) -> Fb {
        let mut fb = Fb { w: 0, h: 0, hbm: null_mut(), bits: null_mut(), dirty: None };
        fb.resize(w, h);
        fb
    }

    pub fn resize(&mut self, w: usize, h: usize) {
        unsafe {
            if !self.hbm.is_null() {
                DeleteObject(self.hbm);
            }
            let bi = dib_info(w.max(1) as i32, h.max(1) as i32);
            let mut bits = null_mut();
            self.hbm = CreateDIBSection(null_mut(), &bi, DIB_RGB_COLORS, &mut bits, null_mut(), 0);
            assert!(!self.hbm.is_null(), "CreateDIBSection failed");
            self.bits = bits as *mut u32;
        }
        self.w = w;
        self.h = h;
        self.dirty = Some([0, 0, w as i32, h as i32]);
    }

    #[inline]
    fn px(&mut self) -> &mut [u32] {
        unsafe { std::slice::from_raw_parts_mut(self.bits, self.w * self.h) }
    }

    /// Copy of the region [x, y, w, h] (clamped to the framebuffer).
    pub fn snapshot(&self, [x, y, w, h]: [usize; 4]) -> crate::img::Image {
        let (x, y) = (x.min(self.w), y.min(self.h));
        let (w, h) = (w.min(self.w - x), h.min(self.h - y));
        let all = unsafe { std::slice::from_raw_parts(self.bits, self.w * self.h) };
        let mut px = Vec::with_capacity(w * h);
        for row in y..y + h {
            px.extend(all[row * self.w + x..row * self.w + x + w].iter().map(|p| p & 0x00ff_ffff));
        }
        crate::img::Image { w, h, px }
    }

    pub fn fits(&self, x: usize, y: usize, w: usize, h: usize) -> bool {
        x + w <= self.w && y + h <= self.h
    }

    pub fn mark(&mut self, x: usize, y: usize, w: usize, h: usize) {
        let r = [x as i32, y as i32, (x + w) as i32, (y + h) as i32];
        self.dirty = Some(match self.dirty {
            None => r,
            Some(d) => [d[0].min(r[0]), d[1].min(r[1]), d[2].max(r[2]), d[3].max(r[3])],
        });
    }

    pub fn raw(&mut self, x: usize, y: usize, w: usize, h: usize, data: &[u8]) {
        let stride = self.w;
        let px = self.px();
        for (j, src) in data.chunks_exact(w * 4).take(h).enumerate() {
            let row = &mut px[(y + j) * stride + x..][..w];
            for (o, p) in row.iter_mut().zip(src.chunks_exact(4)) {
                *o = u32::from_le_bytes([p[0], p[1], p[2], p[3]]);
            }
        }
    }

    pub fn copy(&mut self, sx: usize, sy: usize, dx: usize, dy: usize, w: usize, h: usize) {
        let stride = self.w;
        let px = self.px();
        let mut row = |j: usize| {
            let s = (sy + j) * stride + sx;
            px.copy_within(s..s + w, (dy + j) * stride + dx);
        };
        if dy <= sy {
            (0..h).for_each(&mut row);
        } else {
            (0..h).rev().for_each(&mut row);
        }
    }

    /// Decode one ZRLE rectangle from already-inflated data. None = malformed.
    pub fn zrle(&mut self, d: &[u8], x: usize, y: usize, w: usize, h: usize) -> Option<()> {
        let stride = self.w;
        let px = self.px();
        let mut r = Rd { d, p: 0 };
        let mut pal = [0u32; 128];
        for ty in (y..y + h).step_by(64) {
            let th = 64.min(y + h - ty);
            for tx in (x..x + w).step_by(64) {
                let tw = 64.min(x + w - tx);
                let base = ty * stride + tx;
                let sub = r.u8()?;
                match sub {
                    0 => {
                        let s = r.take(tw * th * 3)?;
                        for (j, src) in s.chunks_exact(tw * 3).enumerate() {
                            let row = &mut px[base + j * stride..][..tw];
                            for (o, c) in row.iter_mut().zip(src.chunks_exact(3)) {
                                *o = cpix(c);
                            }
                        }
                    }
                    1 => {
                        let c = r.pix()?;
                        for j in 0..th {
                            px[base + j * stride..][..tw].fill(c);
                        }
                    }
                    2..=16 => {
                        let n = sub as usize;
                        for e in &mut pal[..n] {
                            *e = r.pix()?;
                        }
                        let bits = match n { 2 => 1, 3..=4 => 2, _ => 4 };
                        let mask = (1u8 << bits) - 1;
                        let rb = (tw * bits + 7) / 8;
                        let s = r.take(rb * th)?;
                        for (j, src) in s.chunks_exact(rb).enumerate() {
                            let row = &mut px[base + j * stride..][..tw];
                            for (i, o) in row.iter_mut().enumerate() {
                                let bit = i * bits;
                                let v = (src[bit / 8] >> (8 - bits - bit % 8)) & mask;
                                *o = pal[v as usize];
                            }
                        }
                    }
                    128 => {
                        let total = tw * th;
                        let mut i = 0;
                        while i < total {
                            let c = r.pix()?;
                            let len = r.run()?;
                            if i + len > total {
                                return None;
                            }
                            fill_run(px, base, stride, tw, i, len, c);
                            i += len;
                        }
                    }
                    130..=255 => {
                        let n = sub as usize - 128;
                        for e in &mut pal[..n] {
                            *e = r.pix()?;
                        }
                        let total = tw * th;
                        let mut i = 0;
                        while i < total {
                            let b = r.u8()?;
                            let c = pal[(b & 127) as usize];
                            let len = if b & 128 != 0 { r.run()? } else { 1 };
                            if i + len > total {
                                return None;
                            }
                            fill_run(px, base, stride, tw, i, len, c);
                            i += len;
                        }
                    }
                    _ => return None,
                }
            }
        }
        Some(())
    }
}

impl Drop for Fb {
    fn drop(&mut self) {
        unsafe { DeleteObject(self.hbm) };
    }
}

#[inline(always)]
fn cpix(c: &[u8]) -> u32 {
    c[0] as u32 | (c[1] as u32) << 8 | (c[2] as u32) << 16
}

#[inline]
fn fill_run(px: &mut [u32], base: usize, stride: usize, tw: usize, mut i: usize, mut len: usize, c: u32) {
    while len > 0 {
        let (row, col) = (i / tw, i % tw);
        let n = (tw - col).min(len);
        let o = base + row * stride + col;
        px[o..o + n].fill(c);
        i += n;
        len -= n;
    }
}

struct Rd<'a> {
    d: &'a [u8],
    p: usize,
}

impl<'a> Rd<'a> {
    #[inline(always)]
    fn u8(&mut self) -> Option<u8> {
        let b = *self.d.get(self.p)?;
        self.p += 1;
        Some(b)
    }
    #[inline(always)]
    fn take(&mut self, n: usize) -> Option<&'a [u8]> {
        let s = self.d.get(self.p..self.p + n)?;
        self.p += n;
        Some(s)
    }
    #[inline(always)]
    fn pix(&mut self) -> Option<u32> {
        self.take(3).map(cpix)
    }
    #[inline(always)]
    fn run(&mut self) -> Option<usize> {
        let mut len = 1;
        loop {
            let b = self.u8()?;
            len += b as usize;
            if b != 255 {
                return Some(len);
            }
        }
    }
}
