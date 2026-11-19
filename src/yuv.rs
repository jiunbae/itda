//! The portable half of the Windows viewer's H.264 decoding (h264dec_win): splitting Annex B
//! into access units, how much data a rectangle may claim, and decoded NV12 pictures to the
//! framebuffer's 0x00RRGGBB pixels. Kept out of the Windows-only module so it is tested on
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

    /// Convert `pic` into `dst` (rows `dst_stride` pixels apart, the first at dst[0]). False,
    /// with nothing written, if a plane or `dst` is too short for the size it claims.
    pub fn convert(&self, pic: &Nv12, dst: &mut [u32], dst_stride: usize) -> bool {
        let (w, h, stride) = (pic.w, pic.h, pic.stride);
        if w == 0 || h == 0 {
            return true;
        }
        let cw = w.div_ceil(2) * 2; // chroma bytes a row reads
        if stride < cw
            || dst_stride < w
            || pic.y.len() < stride * (h - 1) + w
            || pic.uv.len() < stride * (h.div_ceil(2) - 1) + cw
            || dst.len() < dst_stride * (h - 1) + w
        {
            return false;
        }
        for row in 0..h {
            let luma = &pic.y[row * stride..][..w];
            let chroma = &pic.uv[row / 2 * stride..][..cw];
            let out = &mut dst[row * dst_stride..][..w];
            for ((o, l), c) in out.chunks_mut(2).zip(luma.chunks(2)).zip(chroma.chunks_exact(2)) {
                let (u, v) = (c[0] as i32 - 128, c[1] as i32 - 128);
                for (o, &l) in o.iter_mut().zip(l) {
                    *o = self.rgb(l, u, v);
                }
            }
        }
        true
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

    #[test]
    fn rectangle_sizes_are_bounded() {
        assert!(h264_most(3840, 2160) > 3840 * 2160 * 3 / 2);
        assert!(h264_most(0, 0) == 1 << 20);
        assert!(h264_most(4096, 4096) < 70 << 20);
    }
}
