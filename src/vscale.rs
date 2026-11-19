//! Dynamic video resolution for one viewer of the share server.
//!
//! Decoding a 4K picture costs a viewer a lot of memory and CPU, and at the bitrates rate
//! control (rate.rs) settles on most of those pixels carry little detail anyway. Viewers that
//! announce ENC_ITDA_SCALED_VIDEO scale a smaller H.264 picture up to the rectangle, so while
//! the screen moves it is encoded at a size the link's bitrate can fill; the lossless repaint
//! sharpens it at full resolution once it stands still.
//!
//! The size is a step of the screen's: the largest whose pixels fit the budget of about a
//! quarter of the bitrate (bits/s) in pixels, never under 720p's worth. A new size means a
//! new encoder and an IDR, so a change must hold for HOLD, comes at most once every GAP, and
//! a step up needs UP_MARGIN of headroom (unless the bitrate is at its ceiling, where it can
//! only stay or fall: without that, a 4K screen that once dipped would never get its 2/3 back).

use std::time::{Duration, Instant};

/// Scale steps (numerator, denominator), largest first.
pub const STEPS: [(usize, usize); 5] = [(1, 1), (3, 4), (2, 3), (1, 2), (1, 3)];
/// Bits a second per pixel of the budget, as a fraction (3.5): a 4K screen at its 16 Mbit/s
/// goes out at 2/3 (2560x1440), 1080p at its 4.1 Mbit/s at 3/4 (1440x810).
const BITS_PER_PIXEL: (u64, u64) = (7, 2);
/// The budget never drops below this many pixels.
const MIN_BUDGET: u64 = 1280 * 720;
/// How long a new choice must hold before it's taken.
const HOLD: Duration = Duration::from_secs(2);
/// Least time between two changes.
const GAP: Duration = Duration::from_secs(5);
/// Going up needs the budget to exceed the larger size's pixels by this much.
const UP_MARGIN: f64 = 1.2;

/// Pixels a picture of bitrate `bps` can use.
pub fn budget(bps: u32) -> u64 {
    (bps as u64 * BITS_PER_PIXEL.1 / BITS_PER_PIXEL.0).max(MIN_BUDGET)
}

/// The w×h screen at step `i`: rounded, even, at least 16×16.
pub fn scaled(w: usize, h: usize, i: usize) -> (usize, usize) {
    let (n, d) = STEPS[i];
    let f = |v: usize| (((v * n + d / 2) / d) & !1).max(16).min(v & !1);
    (f(w), f(h))
}

fn pixels(w: usize, h: usize, i: usize) -> u64 {
    let (sw, sh) = scaled(w, h, i);
    sw as u64 * sh as u64
}

/// The largest step whose pixels (times `margin`) fit `budget` (the smallest when none does).
fn fit(w: usize, h: usize, budget: u64, margin: f64) -> usize {
    (0..STEPS.len()).find(|&i| pixels(w, h, i) as f64 * margin <= budget as f64).unwrap_or(STEPS.len() - 1)
}

/// The step for a w×h screen at `bps`, without hysteresis.
pub fn choose(w: usize, h: usize, bps: u32) -> usize {
    fit(w, h, budget(bps), 1.0)
}

/// Picks the video size for one viewer over time.
pub struct Scaler {
    src: (usize, usize),
    step: usize,
    /// A change waiting to hold: towards a smaller picture (true) or a larger one, and since.
    pending: Option<(bool, Instant)>,
    changed: Instant,
}

impl Scaler {
    pub fn new(now: Instant) -> Scaler {
        Scaler { src: (0, 0), step: 0, pending: None, changed: now }
    }

    /// The size to encode a w×h screen at, the bitrate being `bps` now (of at most `max`). A
    /// new screen size is chosen for at once; otherwise a change waits for the hysteresis.
    pub fn size(&mut self, w: usize, h: usize, bps: u32, max: u32, now: Instant) -> (usize, usize) {
        if self.src != (w, h) {
            *self = Scaler { src: (w, h), step: choose(w, h, bps), pending: None, changed: now };
            return scaled(w, h, self.step);
        }
        let b = budget(bps);
        let down = fit(w, h, b, 1.0);
        let target = if down > self.step {
            down
        } else if down < self.step {
            fit(w, h, b, if bps >= max { 1.0 } else { UP_MARGIN }).min(self.step)
        } else {
            self.step
        };
        if target == self.step {
            self.pending = None;
        } else {
            let smaller = target > self.step;
            let since = match self.pending {
                Some((s, t)) if s == smaller => t,
                _ => now,
            };
            self.pending = Some((smaller, since));
            if now.duration_since(since) >= HOLD && now.duration_since(self.changed) >= GAP {
                self.step = target;
                self.pending = None;
                self.changed = now;
            }
        }
        scaled(w, h, self.step)
    }
}

/// Area-average (box) downscale of a sw×sh picture (rows of `stride` 0x00RRGGBB pixels) into
/// `dst`, dw×dh rows of `dst_stride`. Each output pixel averages the source pixels its area
/// touches (whole pixels: an edge pixel shared by two outputs counts in both). Large pictures
/// are split into bands over a few threads (4K to 1440p: ~14 ms on one core).
#[allow(clippy::too_many_arguments)]
pub fn downscale(src: &[u32], stride: usize, sw: usize, sh: usize, dst: &mut [u32], dst_stride: usize, dw: usize, dh: usize) {
    assert!(dw <= sw && dh <= sh && dw > 0 && dh > 0 && dst.len() >= (dh - 1) * dst_stride + dw);
    let xs: Vec<(usize, usize)> = (0..dw).map(|x| span(x, sw, dw)).collect();
    let threads = if dw * dh >= 1 << 18 { std::thread::available_parallelism().map_or(1, |n| n.get()).min(4) } else { 1 };
    let band = dh.div_ceil(threads);
    if threads == 1 {
        return rows(src, stride, sw, sh, &xs, dst, dst_stride, dh, 0..dh);
    }
    std::thread::scope(|s| {
        for (i, part) in dst.chunks_mut(band * dst_stride).enumerate() {
            let xs = &xs;
            s.spawn(move || rows(src, stride, sw, sh, xs, part, dst_stride, dh, i * band..((i + 1) * band).min(dh)));
        }
    });
}

/// Source pixels [start, end) that output pixel `i` of `d` covers out of `s`.
fn span(i: usize, s: usize, d: usize) -> (usize, usize) {
    (i * s / d, ((i + 1) * s).div_ceil(d).min(s))
}

/// Output rows `ys` of a downscale into `out` (which starts at row ys.start).
#[allow(clippy::too_many_arguments)]
fn rows(src: &[u32], stride: usize, sw: usize, sh: usize, xs: &[(usize, usize)], out: &mut [u32], dst_stride: usize, dh: usize, ys: std::ops::Range<usize>) {
    // Sums of pixels with B, R and G in 16-bit-apart lanes of a u64 (256 pixels before a lane
    // could overflow; a box is at most 4×4 for these steps).
    let mut col = vec![0u64; sw];
    let start = ys.start;
    for y in ys {
        let (y0, y1) = span(y, sh, dh);
        // The rows' column sums first (contiguous, vectorizes), then each output's columns.
        let first = &src[y0 * stride..y0 * stride + sw];
        for (c, &p) in col.iter_mut().zip(first) {
            *c = (p & 0xff_00ff) as u64 | ((p & 0xff00) as u64) << 24;
        }
        for sy in y0 + 1..y1 {
            let row = &src[sy * stride..sy * stride + sw];
            for (c, &p) in col.iter_mut().zip(row) {
                *c += (p & 0xff_00ff) as u64 | ((p & 0xff00) as u64) << 24;
            }
        }
        let acc = xs.iter().map(|&(x0, x1)| col[x0..x1].iter().sum::<u64>());
        let n_rows = (y1 - y0) as u64;
        let at = (y - start) * dst_stride;
        for ((o, a), &(x0, x1)) in out[at..at + xs.len()].iter_mut().zip(acc).zip(xs) {
            let n = n_rows * (x1 - x0) as u64;
            let half = n / 2;
            let b = ((a & 0xffff) + half) / n;
            let r = (((a >> 16) & 0xffff) + half) / n;
            let g = (((a >> 32) & 0xffff) + half) / n;
            *o = (r << 16 | g << 8 | b) as u32;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn steps_are_even_and_keep_the_aspect() {
        assert_eq!(scaled(3840, 2160, 2), (2560, 1440));
        assert_eq!(scaled(3840, 2160, 4), (1280, 720));
        assert_eq!(scaled(1920, 1080, 1), (1440, 810));
        assert_eq!(scaled(1512, 982, 1), (1134, 736));
        assert_eq!(scaled(1921, 1081, 0), (1920, 1080));
        for (w, h) in [(3840, 2160), (1512, 982), (2560, 1600), (3024, 1964), (1366, 768)] {
            for i in 0..STEPS.len() {
                let (sw, sh) = scaled(w, h, i);
                assert!(sw % 2 == 0 && sh % 2 == 0 && sw <= w && sh <= h);
                let aspect = |a: usize, b: usize| a as f64 / b as f64;
                assert!((aspect(sw, sh) - aspect(w, h)).abs() < 0.01, "{w}x{h} step {i}: {sw}x{sh}");
            }
        }
    }

    #[test]
    fn size_follows_the_bitrate() {
        let pick = |w, h, bps| scaled(w, h, choose(w, h, bps));
        let bitrate = crate::server::video_bitrate;
        assert_eq!(pick(3840, 2160, bitrate(3840, 2160)), (2560, 1440), "4K at 16 Mbit/s: 2/3");
        assert_eq!(pick(3840, 2160, 3_000_000), (1280, 720), "4K pushed to 3 Mbit/s: 1/3");
        assert_eq!(pick(3840, 2160, 1_000_000), (1280, 720), "never under the smallest step");
        assert_eq!(pick(3840, 2160, 40_000_000), (3840, 2160));
        // 1080p at its 4.1 Mbit/s: a budget of 1.18 Mpx, which 3/4 (1.17 Mpx) fits.
        assert_eq!(pick(1920, 1080, bitrate(1920, 1080)), (1440, 810));
        assert_eq!(pick(1920, 1080, 4_700_000), (1440, 810));
        // A screen within the 720p floor is never scaled.
        assert_eq!(pick(1280, 720, 600_000), (1280, 720));
        assert_eq!(pick(1024, 768, bitrate(1024, 768)), (1024, 768));
    }

    #[test]
    fn changes_hold_and_are_spaced() {
        let s = |secs: f64| Duration::from_secs_f64(secs);
        const MAX: u32 = 16_000_000;
        let t0 = Instant::now();
        let mut sc = Scaler::new(t0);
        assert_eq!(sc.size(3840, 2160, MAX, MAX, t0), (2560, 1440), "a new screen: at once");
        // The rate falls: the smaller size only after it held 2 s, and not within 5 s of the start.
        assert_eq!(sc.size(3840, 2160, 3_000_000, MAX, t0 + s(1.0)), (2560, 1440));
        assert_eq!(sc.size(3840, 2160, 3_000_000, MAX, t0 + s(3.5)), (2560, 1440), "held, but 5 s not up");
        assert_eq!(sc.size(3840, 2160, 3_000_000, MAX, t0 + s(5.0)), (1280, 720));
        // A short recovery isn't followed; back at the ceiling, 2/3 again (no margin needed there).
        let t1 = t0 + s(11.0);
        assert_eq!(sc.size(3840, 2160, MAX, MAX, t1), (1280, 720));
        assert_eq!(sc.size(3840, 2160, 3_000_000, MAX, t1 + s(1.0)), (1280, 720));
        assert_eq!(sc.size(3840, 2160, MAX, MAX, t1 + s(2.5)), (1280, 720), "the 2 s start again");
        assert_eq!(sc.size(3840, 2160, MAX, MAX, t1 + s(4.6)), (2560, 1440));
        // Below the ceiling, up needs 20 % headroom: 4K at 3/4 is 4.67 Mpx; 19 Mbit/s (a
        // 5.43 Mpx budget) isn't enough, 23 Mbit/s (6.57 Mpx) is.
        let t2 = t1 + s(20.0);
        assert_eq!(sc.size(3840, 2160, 19_000_000, 40_000_000, t2), (2560, 1440));
        assert_eq!(sc.size(3840, 2160, 19_000_000, 40_000_000, t2 + s(3.0)), (2560, 1440));
        assert_eq!(sc.size(3840, 2160, 23_000_000, 40_000_000, t2 + s(5.0)), (2560, 1440));
        assert_eq!(sc.size(3840, 2160, 23_000_000, 40_000_000, t2 + s(7.0)), (2880, 1620));
        // Below the ceiling and without the headroom: 15 of 20 Mbit/s from 1/3 goes to 1/2 only.
        let mut low = Scaler::new(t0);
        low.size(3840, 2160, 3_000_000, 20_000_000, t0);
        low.size(3840, 2160, 15_000_000, 20_000_000, t0 + s(6.0));
        assert_eq!(low.size(3840, 2160, 15_000_000, 20_000_000, t0 + s(8.0)), (1920, 1080));
        // Falling through several steps is one change of direction: it keeps its 2 s.
        let t3 = t2 + s(20.0);
        assert_eq!(sc.size(3840, 2160, 12_000_000, MAX, t3), (2880, 1620));
        assert_eq!(sc.size(3840, 2160, 3_000_000, MAX, t3 + s(1.0)), (2880, 1620));
        assert_eq!(sc.size(3840, 2160, 3_000_000, MAX, t3 + s(2.0)), (1280, 720));
        // Another screen size starts over at once.
        assert_eq!(sc.size(1920, 1080, MAX, MAX, t3 + s(2.5)), (1920, 1080));
    }

    #[test]
    fn downscale_averages_areas() {
        // 6x4 → 3x2 (½): each output is a 2x2 average.
        let (sw, sh, stride) = (6, 4, 8);
        let mut src = vec![0xdead_beefu32; stride * sh];
        for y in 0..sh {
            for x in 0..sw {
                src[y * stride + x] = ((x * 40) as u32) << 16 | ((y * 60) as u32) << 8 | (x * 10 + y) as u32;
            }
        }
        let mut dst = vec![0u32; 4 * 2];
        downscale(&src, stride, sw, sh, &mut dst, 4, 3, 2);
        for y in 0..2 {
            for x in 0..3 {
                let avg = |f: &dyn Fn(usize, usize) -> u32| {
                    let s: u32 = [(0, 0), (1, 0), (0, 1), (1, 1)].iter().map(|(dx, dy)| f(2 * x + dx, 2 * y + dy)).sum();
                    (s + 2) / 4
                };
                let want = avg(&|x, _| (x * 40) as u32) << 16 | avg(&|_, y| (y * 60) as u32) << 8 | avg(&|x, y| (x * 10 + y) as u32);
                assert_eq!(dst[y * 4 + x], want, "({x},{y})");
            }
            assert_eq!(dst[y * 4 + 3], 0, "past the row untouched");
        }
        // Solid stays solid at any ratio; 2/3 of a 1-px checkerboard is mid grey.
        let solid = vec![0x12_3456u32; 9 * 6];
        let mut out = vec![0u32; 6 * 4];
        downscale(&solid, 9, 9, 6, &mut out, 6, 6, 4);
        assert!(out.iter().all(|&p| p == 0x12_3456));
        let check: Vec<u32> = (0..9 * 6).map(|i| if (i % 9 + i / 9) % 2 == 0 { 0xff_ffff } else { 0 }).collect();
        downscale(&check, 9, 9, 6, &mut out, 6, 6, 4);
        for &p in &out {
            for c in [p >> 16, (p >> 8) & 255, p & 255] {
                assert!((0x50..=0xb0).contains(&c), "{p:06x}");
            }
        }
    }

    #[test]
    fn banded_downscale_matches_one_band() {
        let (w, h) = (1500, 1000);
        let src: Vec<u32> = (0..w * h).map(|i| (i as u32).wrapping_mul(2654435761) & 0xff_ffff).collect();
        let (dw, dh) = (1000, 666);
        assert!(dw * dh >= 1 << 18);
        let mut banded = vec![0u32; dw * dh];
        downscale(&src, w, w, h, &mut banded, dw, dw, dh);
        let xs: Vec<(usize, usize)> = (0..dw).map(|x| span(x, w, dw)).collect();
        let mut one = vec![0u32; dw * dh];
        rows(&src, w, w, h, &xs, &mut one, dw, dh, 0..dh);
        assert!(banded == one);
    }

    /// `cargo test --release downscale_time -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn downscale_time() {
        let (w, h) = (3840, 2160);
        let src: Vec<u32> = (0..w * h).map(|i| (i as u32).wrapping_mul(2654435761) & 0xff_ffff).collect();
        for i in 1..STEPS.len() {
            let (dw, dh) = scaled(w, h, i);
            let mut dst = vec![0u32; dw * dh];
            let t = Instant::now();
            for _ in 0..10 {
                downscale(&src, w, w, h, &mut dst, dw, dw, dh);
            }
            println!("{w}x{h} -> {dw}x{dh}: {:.2} ms", t.elapsed().as_secs_f64() * 100.0);
        }
    }
}
