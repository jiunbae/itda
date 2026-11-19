//! The portable half of the Windows viewer's H.264 decoding (h264dec_win): splitting Annex B
//! into access units, how much data a rectangle may claim, and decoded NV12 pictures to the
//! framebuffer's 0x00RRGGBB pixels, scaled up to their rectangle when the server sent a smaller
//! picture (`Scaler`). Kept out of the Windows-only module so it is tested on
//! every platform.

/// The most bytes an H.264 rectangle of w×h may claim, read before anything is decoded. A
/// key frame of noise can exceed its raw NV12 size a little; four bytes a pixel (what Raw
/// would take) plus 1 MiB is far beyond any real frame and still bounds the allocation.
pub fn h264_most(w: usize, h: usize) -> usize {
    w * h * 4 + (1 << 20)
}

/// Start of each NAL unit: (where its start code begins, where its header byte is).
fn nal_starts(d: &[u8]) -> Vec<(usize, usize)> {
    let mut v = Vec::new();
    let mut i = 0;
    while i + 3 <= d.len() {
        if d[i] == 0 && d[i + 1] == 0 && d[i + 2] == 1 {
            // A four-byte start code's leading zero belongs to it, not to the NAL before.
            let begin = if i > 0 && d[i - 1] == 0 { i - 1 } else { i };
            v.push((begin, i + 3));
            i += 3;
        } else {
            i += 1;
        }
    }
    v
}

/// The access units (pictures) in an Annex B stream, each with its parameter sets and start
/// codes. Media Foundation's decoder is given one picture per sample: the server normally sends
/// one per rectangle, but nothing in the encoding promises that.
pub fn access_units(d: &[u8]) -> Vec<&[u8]> {
    let starts = nal_starts(d);
    let mut units = Vec::new();
    // Where the current unit begins, and whether it already holds a slice.
    let (mut begin, mut has_slice) = (0, false);
    for &(at, hdr) in &starts {
        let Some(&b) = d.get(hdr) else { continue };
        let first_slice = || d.get(hdr + 1).is_some_and(|s| s & 0x80 != 0); // first_mb_in_slice == 0
        let starts_unit = match b & 31 {
            // Access unit delimiter, SEI, SPS, PPS and the reserved prefix types come before
            // the slices of the next picture.
            6..=9 | 14..=18 => true,
            // A slice that begins at macroblock 0 is a new picture's first.
            1 | 5 => first_slice(),
            _ => false,
        };
        if starts_unit && has_slice {
            units.push(&d[begin..at]);
            (begin, has_slice) = (at, false);
        }
        has_slice |= matches!(b & 31, 1 | 5);
    }
    if begin < d.len() {
        units.push(&d[begin..]);
    }
    units
}

/// Whether an Annex B stream holds a NAL unit of `kind` (5 = IDR, 7 = SPS, ...).
pub fn has_nal(d: &[u8], kind: u8) -> bool {
    nal_starts(d).iter().any(|&(_, hdr)| d.get(hdr).is_some_and(|b| b & 31 == kind))
}

/// One decoded picture: a luma plane and the interleaved half-size chroma plane after it.
#[derive(Clone, Copy)]
pub struct Nv12<'a> {
    pub y: &'a [u8],
    pub uv: &'a [u8],
    /// Bytes per row in both planes.
    pub stride: usize,
    pub w: usize,
    pub h: usize,
}

/// YCbCr → RGB in 16.16 fixed point, for one matrix and range.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Yuv {
    y_off: i32,
    y: i32,
    rv: i32,
    gu: i32,
    gv: i32,
    bu: i32,
    pub bt709: bool,
    pub full: bool,
}

impl Yuv {
    /// BT.709 or BT.601; limited (16–235) or full range.
    pub fn new(bt709: bool, full: bool) -> Yuv {
        let (kr, kb) = if bt709 { (0.2126, 0.0722) } else { (0.299, 0.114) };
        let kg = 1.0 - kr - kb;
        let (ys, cs) = if full { (1.0, 1.0) } else { (255.0 / 219.0, 255.0 / 224.0) };
        let fx = |v: f64| (v * 65536.0).round() as i32;
        Yuv {
            y_off: if full { 0 } else { 16 },
            y: fx(ys),
            rv: fx(2.0 * (1.0 - kr) * cs),
            gu: fx(2.0 * (1.0 - kb) * kb / kg * cs),
            gv: fx(2.0 * (1.0 - kr) * kr / kg * cs),
            bu: fx(2.0 * (1.0 - kb) * cs),
            bt709,
            full,
        }
    }

    #[inline(always)]
    fn rgb(&self, y: u8, u: i32, v: i32) -> u32 {
        let l = (y as i32 - self.y_off) * self.y + (1 << 15);
        let r = ((l + self.rv * v) >> 16).clamp(0, 255) as u32;
        let g = ((l - self.gu * u - self.gv * v) >> 16).clamp(0, 255) as u32;
        let b = ((l + self.bu * u) >> 16).clamp(0, 255) as u32;
        r << 16 | g << 8 | b
    }

    /// Whether `pic`'s planes hold the size it claims (checked before any row is read).
    fn planes_fit(pic: &Nv12) -> bool {
        let (w, h, stride) = (pic.w, pic.h, pic.stride);
        let cw = w.div_ceil(2) * 2; // chroma bytes a row reads
        w == 0 || h == 0 || (stride >= cw && pic.y.len() >= stride * (h - 1) + w && pic.uv.len() >= stride * (h.div_ceil(2) - 1) + cw)
    }

    /// One row of `pic` (whose planes fit) into `out` (`pic.w` pixels).
    #[inline]
    fn row(&self, pic: &Nv12, row: usize, out: &mut [u32]) {
        let (w, stride) = (pic.w, pic.stride);
        let cw = w.div_ceil(2) * 2;
        let luma = &pic.y[row * stride..][..w];
        let chroma = &pic.uv[row / 2 * stride..][..cw];
        for ((o, l), c) in out[..w].chunks_mut(2).zip(luma.chunks(2)).zip(chroma.chunks_exact(2)) {
            let (u, v) = (c[0] as i32 - 128, c[1] as i32 - 128);
            for (o, &l) in o.iter_mut().zip(l) {
                *o = self.rgb(l, u, v);
            }
        }
    }

    /// Convert `pic` into `dst` (rows `dst_stride` pixels apart, the first at dst[0]). False,
    /// with nothing written, if a plane or `dst` is too short for the size it claims.
    pub fn convert(&self, pic: &Nv12, dst: &mut [u32], dst_stride: usize) -> bool {
        let (w, h) = (pic.w, pic.h);
        if w == 0 || h == 0 {
            return true;
        }
        if !Self::planes_fit(pic) || dst_stride < w || dst.len() < dst_stride * (h - 1) + w {
            return false;
        }
        for row in 0..h {
            self.row(pic, row, &mut dst[row * dst_stride..]);
        }
        true
    }
}

/// Where each destination pixel of a row (or column) of `to` reads from a row of `from`: the
/// two source pixels and the second one's weight in 1/256 (pixel centres line up, so the edges
/// repeat the source's edge pixels and the same size maps 1:1 with no blending).
fn taps(from: usize, to: usize, out: &mut Vec<(u32, u32, u32)>) {
    out.clear();
    let last = from as i64 - 1;
    out.extend((0..to as i64).map(|d| {
        let pos = (((2 * d + 1) * from as i64) << 16) / (2 * to as i64) - (1 << 15);
        let pos = pos.max(0);
        let (i, f) = (pos >> 16, (pos >> 8) & 255);
        if i >= last { (last as u32, last as u32, 0) } else { (i as u32, i as u32 + 1, f as u32) }
    }));
}

/// a·(1−f) + b·f per 8-bit channel of 0x00RRGGBB, f in 1/256 (0..=255): red and blue in one
/// multiply, green in another.
#[inline(always)]
fn mix(a: u32, b: u32, f: u32) -> u32 {
    let g = 256 - f;
    let rb = ((a & 0xff00ff) * g + (b & 0xff00ff) * f + 0x0080_0080) >> 8;
    let gg = ((a & 0xff00) * g + (b & 0xff00) * f + 0x8000) >> 8;
    (rb & 0xff00ff) | (gg & 0xff00)
}

/// `out` = a·(1−f) + b·f, byte by byte (f in 1/256, 0..=255). In 16-bit lanes, which
/// vectorize on any x86-64 (32-bit multiplies don't before SSE4.1).
fn blend(a: &[u32], b: &[u32], out: &mut [u32], f: u32) {
    let bytes = |p: &[u32]| unsafe { std::slice::from_raw_parts(p.as_ptr() as *const u8, p.len() * 4) };
    let (a, b) = (bytes(a), bytes(b));
    // SAFETY: u8 has no alignment or validity requirements beyond u32's.
    let out = unsafe { std::slice::from_raw_parts_mut(out.as_mut_ptr() as *mut u8, out.len() * 4) };
    let (f, g) = (f as u16, 256 - f as u16);
    for ((o, &p), &q) in out.iter_mut().zip(a).zip(b) {
        *o = ((p as u16 * g + q as u16 * f + 128) >> 8) as u8;
    }
}

/// Draws a decoded picture into its rectangle: as it is when it covers the rectangle (the
/// encoder's macroblock padding is cut off), else scaled to fill it, bilinear. Itda's server
/// sends a smaller picture than the screen area (same aspect) to viewers that asked for it
/// (ENC_ITDA_SCALED_VIDEO), to save bandwidth and both sides' CPU.
///
/// Each picture row is converted once and stretched to the rectangle's width into one of two
/// row buffers; each output row blends the two it falls between. Tables and rows are kept
/// between pictures: nothing is allocated per picture once the sizes settle.
#[derive(Default)]
pub struct Scaler {
    sizes: ((usize, usize), (usize, usize)),
    xs: Vec<(u32, u32, u32)>,
    ys: Vec<(u32, u32, u32)>,
    /// One picture row, converted.
    line: Vec<u32>,
    /// Picture rows stretched to the output width, and which row each holds.
    rows: [Vec<u32>; 2],
    held: [usize; 2],
}

impl Scaler {
    /// Draw `pic` into the w×h rectangle at dst[0] (rows `dst_stride` apart). False, with
    /// nothing written, if a plane or `dst` is too short.
    pub fn draw(&mut self, yuv: &Yuv, pic: &Nv12, dst: &mut [u32], dst_stride: usize, w: usize, h: usize) -> bool {
        if w == 0 || h == 0 {
            return true;
        }
        if pic.w >= w && pic.h >= h {
            return yuv.convert(&Nv12 { w, h, ..*pic }, dst, dst_stride);
        }
        if pic.w == 0 || pic.h == 0 || !Yuv::planes_fit(pic) || dst_stride < w || dst.len() < dst_stride * (h - 1) + w {
            return false;
        }
        let (sw, sh) = (pic.w, pic.h);
        if self.sizes != ((sw, sh), (w, h)) {
            self.sizes = ((sw, sh), (w, h));
            taps(sw, w, &mut self.xs);
            taps(sh, h, &mut self.ys);
            self.line.resize(sw, 0);
            for r in &mut self.rows {
                r.resize(w, 0);
            }
        }
        self.held = [usize::MAX; 2]; // a new picture
        for j in 0..h {
            let (y0, y1, fy) = self.ys[j];
            let a = self.load(yuv, pic, y0 as usize, y1 as usize);
            let b = self.load(yuv, pic, y1 as usize, y0 as usize);
            let out = &mut dst[j * dst_stride..][..w];
            if fy == 0 || a == b {
                out.copy_from_slice(&self.rows[a]);
            } else {
                blend(&self.rows[a], &self.rows[b], out, fy);
            }
        }
        true
    }

    /// The row buffer holding picture row `r` stretched, converting it into the one not
    /// holding `keep` if neither has it yet.
    fn load(&mut self, yuv: &Yuv, pic: &Nv12, r: usize, keep: usize) -> usize {
        if let Some(i) = self.held.iter().position(|&h| h == r) {
            return i;
        }
        let i = if self.held[0] == keep { 1 } else { 0 };
        yuv.row(pic, r, &mut self.line);
        let line = &self.line[..pic.w];
        for (o, &(x0, x1, fx)) in self.rows[i].iter_mut().zip(&self.xs) {
            // SAFETY: `taps` made every index for a row of `pic.w` (< pic.w), and `line` is it.
            let (p, q) = unsafe { (*line.get_unchecked(x0 as usize), *line.get_unchecked(x1 as usize)) };
            *o = mix(p, q, fx);
        }
        self.held[i] = r;
        i
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// NV12 of one colour, h rows `stride` apart.
    fn flat(h: usize, stride: usize, (y, u, v): (u8, u8, u8)) -> (Vec<u8>, Vec<u8>) {
        let luma = vec![y; stride * h];
        let mut chroma = vec![0; stride * h.div_ceil(2)];
        for p in chroma.chunks_exact_mut(2) {
            p.copy_from_slice(&[u, v]);
        }
        (luma, chroma)
    }

    fn near(a: u32, b: u32) -> bool {
        (0..3).all(|i| (((a >> (i * 8)) & 255) as i32 - ((b >> (i * 8)) & 255) as i32).abs() <= 2)
    }

    #[test]
    fn nv12_colours_by_matrix_and_range() {
        // (Y, Cb, Cr) of reference colours, and the 0x00RRGGBB they stand for.
        let cases: [(bool, bool, (u8, u8, u8), u32); 9] = [
            (true, false, (16, 128, 128), 0x000000),
            (true, false, (235, 128, 128), 0xffffff),
            (true, false, (63, 102, 240), 0xff0000),  // BT.709 limited red
            (true, false, (173, 42, 26), 0x00ff00),   // green
            (true, false, (32, 240, 118), 0x0000ff),  // blue
            (false, false, (81, 90, 240), 0xff0000),  // BT.601 limited red
            (false, false, (145, 54, 34), 0x00ff00),
            (true, true, (54, 99, 255), 0xff0000),    // BT.709 full red
            (false, true, (128, 128, 128), 0x808080), // mid grey, any matrix
        ];
        for (bt709, full, yuv, want) in cases {
            let (luma, chroma) = flat(2, 4, yuv);
            let mut dst = [0u32; 8];
            let pic = Nv12 { y: &luma, uv: &chroma, stride: 4, w: 4, h: 2 };
            assert!(Yuv::new(bt709, full).convert(&pic, &mut dst, 4));
            assert!(dst.iter().all(|&p| near(p, want)), "{yuv:?} 709={bt709} full={full}: {:06x} != {want:06x}", dst[0]);
        }
    }

    #[test]
    fn nv12_shares_chroma_per_2x2_and_honours_strides() {
        // 3×3 with padded rows: odd sizes read the last chroma pair for the last column/row.
        let stride = 8;
        let mut luma = vec![0u8; stride * 3];
        let mut chroma = vec![128u8; stride * 2];
        for r in 0..3 {
            luma[r * stride..r * stride + 3].copy_from_slice(&[16, 126, 235]);
        }
        chroma[2..4].copy_from_slice(&[128, 240]); // right column, top pair of rows: reddish
        let mut dst = vec![0xdead_beefu32; 5 * 3];
        let yuv = Yuv::new(true, false);
        assert!(yuv.convert(&Nv12 { y: &luma, uv: &chroma, stride, w: 3, h: 3 }, &mut dst, 5));
        assert_eq!(dst[0], 0x000000);
        assert!(near(dst[1], 0x808080), "{:06x}", dst[1]);
        assert_eq!(dst[2] >> 16, 255, "red from the second chroma pair: {:06x}", dst[2]);
        assert_eq!(dst[5 + 2] >> 16, 255, "row 1 shares row 0's chroma");
        assert!(near(dst[10 + 2], 0xffffff), "row 2 has its own (neutral) chroma");
        assert_eq!(dst[3..5], [0xdead_beef; 2], "outside the picture stays untouched");
    }

    #[test]
    fn nv12_refuses_short_planes() {
        let (luma, chroma) = flat(4, 4, (16, 128, 128));
        let yuv = Yuv::new(true, false);
        let mut dst = vec![0u32; 16];
        assert!(!yuv.convert(&Nv12 { y: &luma[..12], uv: &chroma, stride: 4, w: 4, h: 4 }, &mut dst, 4));
        assert!(!yuv.convert(&Nv12 { y: &luma, uv: &chroma[..4], stride: 4, w: 4, h: 4 }, &mut dst, 4));
        assert!(!yuv.convert(&Nv12 { y: &luma, uv: &chroma, stride: 4, w: 4, h: 4 }, &mut dst[..15], 4));
        assert!(!yuv.convert(&Nv12 { y: &luma, uv: &chroma, stride: 3, w: 4, h: 4 }, &mut dst, 4));
        assert!(dst.iter().all(|&p| p == 0));
    }

    #[test]
    fn splits_access_units() {
        let sps = [0, 0, 0, 1, 0x67, 0x64, 0x00, 0x28];
        let pps = [0, 0, 0, 1, 0x68, 0xee, 0x3c];
        let idr = [0, 0, 0, 1, 0x65, 0x88, 0x84, 0x00];
        let idr2 = [0, 0, 1, 0x65, 0x1f, 0x84]; // a second slice of the same picture (first_mb != 0)
        let p1 = [0, 0, 0, 1, 0x41, 0x9a, 0x01];
        let p2 = [0, 0, 1, 0x41, 0x9b, 0x02];
        let all: Vec<u8> = [&sps[..], &pps, &idr, &idr2, &p1, &p2].concat();
        let units = access_units(&all);
        assert_eq!(units.len(), 3, "{units:?}");
        assert_eq!(units[0], [&sps[..], &pps, &idr, &idr2].concat());
        assert_eq!(units[1], p1);
        assert_eq!(units[2], p2);
        assert!(has_nal(&all, 5) && has_nal(&all, 7) && has_nal(&all, 8) && !has_nal(&all, 6));
        // Parameter sets after a picture begin the next one.
        let again: Vec<u8> = [&p1[..], &sps, &pps, &idr].concat();
        assert_eq!(access_units(&again), [&p1[..], &[&sps[..], &pps, &idr].concat()]);
        // Nothing recognisable is passed on whole; nothing at all yields nothing.
        assert_eq!(access_units(&[1, 2, 3]), [&[1u8, 2, 3][..]]);
        assert!(access_units(&[]).is_empty());
    }

    /// RGB "NV12": neutral chroma, so each pixel is its luma as grey (full range: Y = value).
    fn grey(w: usize, h: usize, stride: usize, f: impl Fn(usize, usize) -> u8) -> (Vec<u8>, Vec<u8>) {
        let mut luma = vec![0u8; stride * h];
        for y in 0..h {
            for x in 0..w {
                luma[y * stride + x] = f(x, y);
            }
        }
        (luma, vec![128u8; stride * h.div_ceil(2)])
    }

    #[test]
    fn scaler_same_size_is_the_plain_conversion() {
        let (w, h, stride) = (7, 5, 8);
        let (luma, chroma) = grey(w, h, stride, |x, y| (x * 30 + y * 7) as u8);
        let pic = Nv12 { y: &luma, uv: &chroma, stride, w, h };
        let yuv = Yuv::new(true, true);
        let mut plain = vec![1u32; 9 * h];
        assert!(yuv.convert(&pic, &mut plain, 9));
        let mut scaled = vec![1u32; 9 * h];
        assert!(Scaler::default().draw(&yuv, &pic, &mut scaled, 9, w, h));
        assert_eq!(plain, scaled);
        // A bigger picture (macroblock padding) is cut to the rectangle, not shrunk.
        let mut cut = vec![1u32; 9 * 3];
        assert!(Scaler::default().draw(&yuv, &pic, &mut cut, 9, 4, 3));
        for y in 0..3 {
            assert_eq!(cut[y * 9..y * 9 + 4], plain[y * 9..y * 9 + 4]);
            assert_eq!(cut[y * 9 + 4], 1, "nothing past the rectangle");
        }
    }

    #[test]
    fn scaler_fills_the_rectangle_bilinear() {
        // 2×2 → 4×4: pixel centres line up, so the edges repeat the corner pixels and the
        // middle blends a quarter / three quarters.
        let (luma, chroma) = grey(2, 2, 2, |x, y| [[0u8, 200], [100, 40]][y][x]);
        let pic = Nv12 { y: &luma, uv: &chroma, stride: 2, w: 2, h: 2 };
        let yuv = Yuv::new(true, true);
        let mut dst = vec![0xdead_beefu32; 6 * 5];
        let mut sc = Scaler::default();
        assert!(sc.draw(&yuv, &pic, &mut dst, 6, 4, 4));
        let at = |x: usize, y: usize| dst[y * 6 + x] & 255;
        assert_eq!([at(0, 0), at(3, 0), at(0, 3), at(3, 3)], [0, 200, 100, 40], "corners");
        assert_eq!([at(1, 0), at(2, 0)], [50, 150], "top row: 1/4 and 3/4 of the way");
        assert_eq!([at(0, 1), at(0, 2)], [25, 75], "left column");
        // (0,0)…(200,40) both ways at a quarter: 0.75·(0.75·0 + 0.25·200) + 0.25·(0.75·100 + 0.25·40)
        assert!((at(1, 1) as i32 - 59).abs() <= 1, "{}", at(1, 1));
        assert!(dst.chunks(6).take(4).all(|r| r[4..] == [0xdead_beef; 2]), "rows end at the rectangle");
        assert_eq!(dst[24..], [0xdead_beef; 6], "nothing below it");
        // The same scaler again, then for another size: same result, new tables.
        let first = dst.clone();
        assert!(sc.draw(&yuv, &pic, &mut dst, 6, 4, 4));
        assert_eq!(dst, first);
        // One row: halfway between the picture's rows (50, 120), stretched ×3 across.
        let mut wide = [0u32; 6];
        assert!(sc.draw(&yuv, &pic, &mut wide, 6, 6, 1));
        assert_eq!(wide.map(|p| p & 255), [50, 50, 73, 97, 120, 120]);
    }

    #[test]
    fn scaler_exact_sizes_and_short_buffers() {
        // 2560×1440 → 3840×2160 (×1.5), and an odd ratio; every output pixel written, none past.
        for (sw, sh, w, h) in [(64, 36, 96, 54), (10, 6, 33, 19), (1, 1, 5, 3)] {
            let (luma, chroma) = grey(sw, sh, sw + 2, |x, y| ((x + y) * 3) as u8);
            let pic = Nv12 { y: &luma, uv: &chroma, stride: sw + 2, w: sw, h: sh };
            let stride = w + 1;
            let mut dst = vec![0xdead_beefu32; stride * h + 1];
            assert!(Scaler::default().draw(&Yuv::new(true, false), &pic, &mut dst, stride, w, h));
            for y in 0..h {
                assert!(dst[y * stride..][..w].iter().all(|&p| p >> 24 == 0), "{sw}x{sh}→{w}x{h} row {y}");
                assert_eq!(dst[y * stride + w], 0xdead_beef);
            }
            assert_eq!(dst[stride * h], 0xdead_beef);
            let mut short = vec![0u32; stride * (h - 1) + w - 1];
            assert!(!Scaler::default().draw(&Yuv::new(true, false), &pic, &mut short, stride, w, h));
            assert!(short.iter().all(|&p| p == 0));
        }
        let (luma, chroma) = grey(4, 4, 4, |_, _| 9);
        let pic = Nv12 { y: &luma[..10], uv: &chroma, stride: 4, w: 4, h: 4 };
        assert!(!Scaler::default().draw(&Yuv::new(true, false), &pic, &mut vec![0; 64], 8, 8, 8));
    }

    /// How long a 2560×1440 picture takes to fill 3840×2160 (`cargo test --release -- --ignored
    /// --nocapture scaler_speed`).
    #[test]
    #[ignore]
    fn scaler_speed() {
        let (sw, sh, w, h) = (2560, 1440, 3840, 2160);
        let (luma, chroma) = grey(sw, sh, sw, |x, y| (x ^ y) as u8);
        let pic = Nv12 { y: &luma, uv: &chroma, stride: sw, w: sw, h: sh };
        let yuv = Yuv::new(true, false);
        let mut dst = vec![0u32; w * h];
        let mut sc = Scaler::default();
        let mut plain = vec![0u32; sw * sh];
        for (name, scaled) in [("convert 2560x1440", false), ("scale to 3840x2160", true)] {
            let t = std::time::Instant::now();
            for _ in 0..20 {
                if scaled { assert!(sc.draw(&yuv, &pic, &mut dst, w, w, h)) } else { assert!(yuv.convert(&pic, &mut plain, sw)) }
            }
            eprintln!("{name}: {:.1} ms", t.elapsed().as_secs_f64() * 1000.0 / 20.0);
        }
    }

    #[test]
    fn rectangle_sizes_are_bounded() {
        assert!(h264_most(3840, 2160) > 3840 * 2160 * 3 / 2);
        assert!(h264_most(0, 0) == 1 << 20);
        assert!(h264_most(4096, 4096) < 70 << 20);
    }
}
