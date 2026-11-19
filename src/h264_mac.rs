//! Hardware H.264 (VideoToolbox) for RFB encoding 50 ("Open H.264"): one encoder per viewer,
//! fed 0x00RRGGBB frames (BGRA in memory), giving Annex B access units. Real-time, no frame
//! reordering, low-latency rate control; key frames carry SPS and PPS so a decoder can start
//! (or restart) from any of them.
//!
//! The encoded picture may be smaller than the frames it's given (dynamic video resolution,
//! vscale.rs): captured buffers are scaled by a VTPixelTransferSession (on the GPU), CPU
//! frames by an area-average downscale, into the encoder's own buffer. (VTCompressionSession
//! does take a larger buffer than its size without an error, and doesn't simply crop it, but
//! what it does with it isn't documented: see `session_given_other_sizes`.)
use objc2_core_foundation::{CFBoolean, CFDictionary, CFNumber, CFRetained, CFString, CFType};
use objc2_core_media::{kCMTimeInvalid, CMFormatDescription, CMSampleBuffer, CMTime, CMVideoFormatDescriptionGetH264ParameterSetAtIndex};
use objc2_core_video::{kCVPixelFormatType_32BGRA, CVPixelBuffer, CVPixelBufferCreate, CVPixelBufferGetBaseAddress, CVPixelBufferGetBytesPerRow, CVPixelBufferGetHeight, CVPixelBufferGetWidth, CVPixelBufferLockBaseAddress, CVPixelBufferLockFlags, CVPixelBufferUnlockBaseAddress};
use objc2_video_toolbox::*;
use std::ffi::c_void;
use std::ptr::{null_mut, NonNull};
use std::sync::Mutex;
use std::time::Instant;

const CODEC_H264: u32 = u32::from_be_bytes(*b"avc1");

/// Encoded output of one frame: Annex B NAL units, and whether it starts a new GOP.
pub struct Unit {
    pub data: Vec<u8>,
    pub key: bool,
}

pub struct H264 {
    session: CFRetained<VTCompressionSession>,
    out: Box<Mutex<Vec<Unit>>>,
    buffer: CFRetained<CVPixelBuffer>,
    pub width: usize,
    pub height: usize,
    /// The size of the frames it's given (larger than width x height when scaling).
    pub src: (usize, usize),
    /// Scales captured buffers of another size (made when first needed).
    transfer: Option<CFRetained<VTPixelTransferSession>>,
    clock: FrameClock,
}

/// Rate control must see actual submission time. A fixed frame/60 clock compresses
/// time whenever capture, encoding or backpressure reduces the frame rate, spending
/// a 60-fps bit budget on a much slower stream (and unnecessarily dropping pictures).
#[derive(Default)]
struct FrameClock {
    start: Option<Instant>,
    last: i64,
}

impl FrameClock {
    fn next(&mut self, now: Instant) -> i64 {
        let Some(start) = self.start else {
            self.start = Some(now);
            return 0;
        };
        let elapsed = now.saturating_duration_since(start).as_micros().min((i64::MAX - 1) as u128) as i64;
        self.last = elapsed.max(self.last + 1);
        self.last
    }
}

// The session and pixel buffer are used by one viewer thread at a time.
unsafe impl Send for H264 {}

fn set(session: &VTCompressionSession, key: &CFString, value: &CFType) -> bool {
    let s = session as *const VTCompressionSession as *const VTSession;
    unsafe { VTSessionSetProperty(&*s, key, Some(value)) == 0 }
}

/// A transfer session that scales a whole buffer into a whole buffer, averaging (not
/// skipping) pixels when it shrinks, without waiting for other GPU work.
fn transfer_session() -> Result<CFRetained<VTPixelTransferSession>, String> {
    let mut raw: *mut VTPixelTransferSession = null_mut();
    let st = unsafe { VTPixelTransferSession::create(None, NonNull::from(&mut raw)) };
    let t = NonNull::new(raw).filter(|_| st == 0).ok_or_else(|| format!("VTPixelTransferSessionCreate {st}"))?;
    let t = unsafe { CFRetained::from_raw(t) };
    let vs = &*t as *const VTPixelTransferSession as *const VTSession;
    unsafe {
        VTSessionSetProperty(&*vs, kVTPixelTransferPropertyKey_ScalingMode, Some(kVTScalingMode_Normal));
        VTSessionSetProperty(&*vs, kVTPixelTransferPropertyKey_DownsamplingMode, Some(kVTDownsamplingMode_Average));
        VTSessionSetProperty(&*vs, kVTPixelTransferPropertyKey_RealTime, Some(CFBoolean::new(true)));
    }
    Ok(t)
}

impl H264 {
    /// An encoder for width x height frames (even sizes) at about `bitrate` bits a second.
    pub fn new(width: usize, height: usize, bitrate: i64) -> Result<H264, String> {
        H264::scaled(width, height, width, height, bitrate)
    }

    /// An encoder for src_w x src_h frames that encodes them at width x height (rounded down
    /// to even, at most the frames' size).
    pub fn scaled(src_w: usize, src_h: usize, width: usize, height: usize, bitrate: i64) -> Result<H264, String> {
        let (width, height) = (width.min(src_w) & !1, height.min(src_h) & !1);
        if width < 16 || height < 16 {
            return Err("frame too small".into());
        }
        let out: Box<Mutex<Vec<Unit>>> = Box::new(Mutex::new(Vec::new()));
        let mut raw: *mut VTCompressionSession = null_mut();
        let spec = unsafe {
            CFDictionary::<CFString, CFType>::from_slices(
                &[kVTVideoEncoderSpecification_EnableLowLatencyRateControl],
                &[CFBoolean::new(true).as_ref()],
            )
        };
        let st = unsafe {
            VTCompressionSession::create(
                None,
                width as i32,
                height as i32,
                CODEC_H264,
                Some(spec.as_opaque()),
                None,
                None,
                Some(output),
                &*out as *const Mutex<Vec<Unit>> as *mut c_void,
                NonNull::from(&mut raw),
            )
        };
        let session = NonNull::new(raw).filter(|_| st == 0).ok_or_else(|| format!("VTCompressionSessionCreate {st}"))?;
        let session = unsafe { CFRetained::from_raw(session) };
        unsafe {
            set(&session, kVTCompressionPropertyKey_RealTime, CFBoolean::new(true));
            set(&session, kVTCompressionPropertyKey_AllowFrameReordering, CFBoolean::new(false));
            set(&session, kVTCompressionPropertyKey_ProfileLevel, kVTProfileLevel_H264_High_AutoLevel);
            set(&session, kVTCompressionPropertyKey_AverageBitRate, &CFNumber::new_i64(bitrate));
            // Key frames only when asked (join, reset, resize): screens are mostly still.
            set(&session, kVTCompressionPropertyKey_MaxKeyFrameInterval, &CFNumber::new_i32(i32::MAX));
            set(&session, kVTCompressionPropertyKey_ExpectedFrameRate, &CFNumber::new_i32(60));
            // One reference picture: a live screen never looks further back, and the stream
            // then tells decoders so (max_dec_frame_buffering). Left alone it says 12, and a
            // Windows viewer's decoder kept that many pictures: 113 MB at 2560x1440.
            // (kVTCompressionPropertyKey_ReferenceBufferCount, by name: macOS 13 and later;
            // older ones ignore it.)
            set(&session, &CFString::from_static_str("ReferenceBufferCount"), &CFNumber::new_i32(1));
            session.prepare_to_encode_frames();
        }
        let mut pb: *mut CVPixelBuffer = null_mut();
        let attrs = unsafe {
            CFDictionary::<CFString, CFType>::from_slices(&[objc2_core_video::kCVPixelBufferIOSurfacePropertiesKey], &[CFDictionary::<CFString, CFType>::empty().as_ref()])
        };
        let r = unsafe { CVPixelBufferCreate(None, width, height, kCVPixelFormatType_32BGRA, Some(attrs.as_opaque()), NonNull::from(&mut pb)) };
        let buffer = NonNull::new(pb).filter(|_| r == 0).ok_or_else(|| format!("CVPixelBufferCreate {r}"))?;
        Ok(H264 { session, out, buffer: unsafe { CFRetained::from_raw(buffer) }, width, height, src: (src_w, src_h), transfer: None, clock: FrameClock::default() })
    }

    /// Aim for `bitrate` bits a second from the next frame on (a running session accepts it).
    pub fn set_bitrate(&mut self, bitrate: i64) -> bool {
        unsafe { set(&self.session, kVTCompressionPropertyKey_AverageBitRate, &CFNumber::new_i64(bitrate)) }
    }

    /// Encode one frame (`px`: rows of `stride` pixels, of the frames' size `src`); `key` forces
    /// an IDR with parameter sets. Waits for the encoder, so the result is this frame's.
    pub fn encode(&mut self, px: &[u32], stride: usize, key: bool) -> Result<Unit, String> {
        let (sw, sh) = self.src;
        let scale = (self.width, self.height) != (sw & !1, sh & !1);
        unsafe {
            let pb = &*self.buffer;
            CVPixelBufferLockBaseAddress(pb, CVPixelBufferLockFlags(0));
            let base = CVPixelBufferGetBaseAddress(pb) as *mut u8;
            let bpr = CVPixelBufferGetBytesPerRow(pb);
            if scale {
                assert!(px.len() >= (sh - 1) * stride + sw && bpr.is_multiple_of(4));
                let dst = std::slice::from_raw_parts_mut(base as *mut u32, bpr / 4 * self.height);
                crate::vscale::downscale(px, stride, sw, sh, dst, bpr / 4, self.width, self.height);
            } else {
                for y in 0..self.height {
                    let src = &px[y * stride..y * stride + self.width];
                    std::ptr::copy_nonoverlapping(src.as_ptr() as *const u8, base.add(y * bpr), self.width * 4);
                }
            }
            CVPixelBufferUnlockBaseAddress(pb, CVPixelBufferLockFlags(0));
        }
        self.encode_ready(&self.buffer.clone(), key)
    }

    /// Encode a ready pixel buffer (e.g. a captured IOSurface), of this encoder's size or, when
    /// it scales, of the frames' size.
    pub fn encode_buffer(&mut self, pb: &CVPixelBuffer, key: bool) -> Result<Unit, String> {
        let size = (CVPixelBufferGetWidth(pb), CVPixelBufferGetHeight(pb));
        // (An odd-sized capture goes in as it is, as before scaling existed.)
        if (size.0 & !1, size.1 & !1) == (self.width, self.height) {
            return self.encode_ready(pb, key);
        }
        if self.transfer.is_none() {
            self.transfer = Some(transfer_session()?);
        }
        let st = unsafe { self.transfer.as_ref().unwrap().transfer_image(pb, &self.buffer) };
        if st != 0 {
            return Err(format!("VTPixelTransferSessionTransferImage {st}"));
        }
        self.encode_ready(&self.buffer.clone(), key)
    }

    fn encode_ready(&mut self, pb: &CVPixelBuffer, key: bool) -> Result<Unit, String> {
        let pts = unsafe { CMTime::new(self.clock.next(Instant::now()), 1_000_000) };
        let props = key.then(|| unsafe {
            CFDictionary::<CFString, CFType>::from_slices(&[kVTEncodeFrameOptionKey_ForceKeyFrame], &[CFBoolean::new(true).as_ref()])
        });
        let st = unsafe {
            self.session.encode_frame(pb, pts, kCMTimeInvalid, props.as_ref().map(|d| d.as_opaque()), null_mut(), null_mut())
        };
        if st != 0 {
            return Err(format!("VTCompressionSessionEncodeFrame {st}"));
        }
        unsafe { self.session.complete_frames(pts) };
        let mut q = self.out.lock().unwrap();
        let mut unit = Unit { data: Vec::new(), key: false };
        for u in q.drain(..) {
            unit.key |= u.key;
            unit.data.extend_from_slice(&u.data);
        }
        Ok(unit)
    }
}

impl Drop for H264 {
    fn drop(&mut self) {
        if let Some(t) = &self.transfer {
            unsafe { t.invalidate() };
        }
        unsafe { self.session.invalidate() };
    }
}

const START: [u8; 4] = [0, 0, 0, 1];

/// VideoToolbox's callback: AVCC (length-prefixed) to Annex B, with SPS/PPS before an IDR.
unsafe extern "C-unwind" fn output(refcon: *mut c_void, _: *mut c_void, status: i32, _: VTEncodeInfoFlags, sample: *mut CMSampleBuffer) {
    let Some(sample) = (unsafe { sample.as_ref() }) else { return };
    if status != 0 {
        return;
    }
    let out = unsafe { &*(refcon as *const Mutex<Vec<Unit>>) };
    let Some(block) = (unsafe { sample.data_buffer() }) else { return };
    let len = unsafe { block.data_length() };
    let mut avcc = vec![0u8; len];
    if unsafe { block.copy_data_bytes(0, len, NonNull::new_unchecked(avcc.as_mut_ptr() as *mut c_void)) } != 0 {
        return;
    }
    let mut nals = Vec::new();
    let mut i = 0;
    while i + 4 <= avcc.len() {
        let n = u32::from_be_bytes([avcc[i], avcc[i + 1], avcc[i + 2], avcc[i + 3]]) as usize;
        if i + 4 + n > avcc.len() {
            break;
        }
        nals.push(&avcc[i + 4..i + 4 + n]);
        i += 4 + n;
    }
    let key = nals.iter().any(|n| !n.is_empty() && n[0] & 0x1f == 5);
    let mut data = Vec::with_capacity(len + 64);
    if key {
        if let Some(fd) = unsafe { sample.format_description() } {
            for idx in 0..2 {
                if let Some(ps) = parameter_set(&fd, idx) {
                    data.extend_from_slice(&START);
                    data.extend_from_slice(ps);
                }
            }
        }
    }
    for n in nals {
        data.extend_from_slice(&START);
        data.extend_from_slice(n);
    }
    out.lock().unwrap().push(Unit { data, key });
}

fn parameter_set(fd: &CMFormatDescription, index: usize) -> Option<&[u8]> {
    let (mut p, mut size) = (std::ptr::null::<u8>(), 0usize);
    let st = unsafe { CMVideoFormatDescriptionGetH264ParameterSetAtIndex(fd, index, &mut p, &mut size, null_mut(), null_mut()) };
    (st == 0 && !p.is_null()).then(|| unsafe { std::slice::from_raw_parts(p, size) })
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    #[test]
    fn encoder_time_tracks_slow_frames_and_resume_after_idle() {
        use std::time::Duration;
        let start = Instant::now();
        let mut clock = FrameClock::default();
        assert_eq!(clock.next(start), 0);
        // A 30 fps source must not be described as 60 fps to rate control.
        assert_eq!(clock.next(start + Duration::from_micros(33_333)), 33_333);
        assert_eq!(clock.next(start + Duration::from_micros(66_666)), 66_666);
        // Backpressure/idle is elapsed media time too, not one more 1/60 second.
        assert_eq!(clock.next(start + Duration::from_secs(2)), 2_000_000);
        assert_eq!(clock.next(start + Duration::from_micros(2_016_667)), 2_016_667);
        // Rapid submissions/retries still have strictly increasing timestamps.
        assert_eq!(clock.next(start + Duration::from_micros(2_016_667)), 2_016_668);
    }

    /// NAL unit types in an Annex B stream.
    fn types(b: &[u8]) -> Vec<u8> {
        let mut t = Vec::new();
        let mut i = 0;
        while i + 4 < b.len() {
            if b[i..i + 4] == START {
                t.push(b[i + 4] & 0x1f);
                i += 4;
            } else {
                i += 1;
            }
        }
        t
    }

    /// A running session takes a new bitrate (the share server's rate control) and its
    /// output shrinks accordingly.
    #[test]
    fn bitrate_changes_while_running() {
        let (w, h) = (640, 360);
        let mut e = H264::new(w, h, 8_000_000).expect("encoder");
        let mut seed = 1u32;
        let mut frames = |e: &mut H264, n: usize| -> usize {
            (0..n).map(|i| {
                let px: Vec<u32> = (0..w * h).map(|_| { seed = seed.wrapping_mul(1664525).wrapping_add(1013904223); seed >> 8 }).collect();
                e.encode(&px, w, i == 0).map(|u| u.data.len()).unwrap_or(0)
            }).sum()
        };
        let high = frames(&mut e, 30);
        assert!(e.set_bitrate(500_000));
        let low = frames(&mut e, 30);
        assert!(low < high / 2, "{low} bytes at 0.5 Mbit/s vs {high} at 8");
    }

    #[test]
    fn key_frames_carry_parameter_sets() {
        let (w, h) = (640, 360);
        let mut e = H264::new(w, h, 4_000_000).expect("encoder");
        let mut px: Vec<u32> = (0..w * h).map(|i| (i as u32).wrapping_mul(2654435761) & 0xff_ffff).collect();
        let first = e.encode(&px, w, true).unwrap();
        assert!(first.key);
        assert_eq!(&types(&first.data)[..2], &[7, 8], "SPS, PPS first: {:?}", types(&first.data));
        assert!(types(&first.data).contains(&5));
        px[1000] ^= 0xffffff;
        let next = e.encode(&px, w, false).unwrap();
        assert!(!next.key && !types(&next.data).contains(&7), "{:?}", types(&next.data));
        assert!(next.data.len() < first.data.len() / 4, "an almost unchanged frame should be small");
        let again = e.encode(&px, w, true).unwrap();
        assert!(again.key && types(&again.data)[..2] == [7, 8]);
    }

    /// Exp-Golomb reader over an RBSP.
    struct Bits<'a>(&'a [u8], usize);
    impl Bits<'_> {
        fn bit(&mut self) -> u32 {
            let b = (self.0[self.1 / 8] >> (7 - self.1 % 8)) & 1;
            self.1 += 1;
            b as u32
        }
        fn bits(&mut self, n: u32) -> u32 {
            (0..n).fold(0, |v, _| v << 1 | self.bit())
        }
        fn ue(&mut self) -> u32 {
            let mut zeros = 0;
            while self.bit() == 0 {
                zeros += 1;
            }
            (1 << zeros) - 1 + self.bits(zeros)
        }
        fn se(&mut self) -> i32 {
            let v = self.ue() as i32;
            if v % 2 == 1 { (v + 1) / 2 } else { -v / 2 }
        }
    }

    /// The picture size an Annex B stream's SPS declares (after its cropping).
    pub(crate) fn sps_size(b: &[u8]) -> Option<(usize, usize)> {
        let at = (0..b.len().saturating_sub(4)).find(|&i| b[i..i + 4] == START && b[i + 4] & 0x1f == 7)? + 5;
        let end = (at..b.len().saturating_sub(3)).find(|&i| b[i..i + 4] == START).unwrap_or(b.len());
        // Emulation prevention bytes out.
        let mut rbsp = Vec::new();
        for &x in &b[at..end] {
            if x == 3 && rbsp.ends_with(&[0, 0]) {
                continue;
            }
            rbsp.push(x);
        }
        let mut r = Bits(&rbsp, 0);
        let profile = r.bits(8);
        r.bits(16); // constraint flags, level
        r.ue(); // sps id
        let mut chroma = 1;
        if [100, 110, 122, 244, 44, 83, 86, 118, 128, 138, 139, 134, 135].contains(&profile) {
            chroma = r.ue();
            if chroma == 3 {
                r.bit();
            }
            r.ue();
            r.ue();
            r.bit();
            if r.bit() == 1 {
                for i in 0..if chroma == 3 { 12 } else { 8 } {
                    if r.bit() == 1 {
                        let (mut last, mut next) = (8i32, 8i32);
                        for _ in 0..if i < 6 { 16 } else { 64 } {
                            if next != 0 {
                                next = (last + r.se() + 256) % 256;
                            }
                            last = if next == 0 { last } else { next };
                        }
                    }
                }
            }
        }
        r.ue(); // log2_max_frame_num
        match r.ue() {
            0 => { r.ue(); }
            1 => {
                r.bit();
                r.se();
                r.se();
                for _ in 0..r.ue() {
                    r.se();
                }
            }
            _ => {}
        }
        r.ue(); // max refs
        r.bit();
        let w = (r.ue() + 1) * 16;
        let map_units = r.ue() + 1;
        let frame_mbs_only = r.bit();
        let h = map_units * 16 * (2 - frame_mbs_only);
        if frame_mbs_only == 0 {
            r.bit();
        }
        r.bit();
        let (mut cw, mut ch) = (0, 0);
        if r.bit() == 1 {
            let (sx, sy) = if chroma == 0 { (1, 2 - frame_mbs_only) } else { (2, 2 * (2 - frame_mbs_only)) };
            cw = (r.ue() + r.ue()) * sx;
            ch = (r.ue() + r.ue()) * sy;
        }
        Some(((w - cw) as usize, (h - ch) as usize))
    }

    /// A BGRA IOSurface buffer filled by `f(x, y)`.
    fn surface(w: usize, h: usize, f: impl Fn(usize, usize) -> u32) -> CFRetained<CVPixelBuffer> {
        let mut pb: *mut CVPixelBuffer = null_mut();
        let attrs = unsafe {
            CFDictionary::<CFString, CFType>::from_slices(&[objc2_core_video::kCVPixelBufferIOSurfacePropertiesKey], &[CFDictionary::<CFString, CFType>::empty().as_ref()])
        };
        assert_eq!(unsafe { CVPixelBufferCreate(None, w, h, kCVPixelFormatType_32BGRA, Some(attrs.as_opaque()), NonNull::from(&mut pb)) }, 0);
        let pb = unsafe { CFRetained::from_raw(NonNull::new(pb).unwrap()) };
        unsafe {
            CVPixelBufferLockBaseAddress(&pb, CVPixelBufferLockFlags(0));
            let base = CVPixelBufferGetBaseAddress(&pb) as *mut u32;
            let stride = CVPixelBufferGetBytesPerRow(&pb) / 4;
            for y in 0..h {
                for x in 0..w {
                    *base.add(y * stride + x) = f(x, y);
                }
            }
            CVPixelBufferUnlockBaseAddress(&pb, CVPixelBufferLockFlags(0));
        }
        pb
    }

    /// The encoder's own buffer, read back (what it was last given to encode).
    fn staged(e: &H264) -> Vec<u32> {
        let pb = &*e.buffer;
        unsafe {
            CVPixelBufferLockBaseAddress(pb, CVPixelBufferLockFlags::ReadOnly);
            let base = CVPixelBufferGetBaseAddress(pb) as *const u32;
            let stride = CVPixelBufferGetBytesPerRow(pb) / 4;
            let v = (0..e.height).flat_map(|y| (0..e.width).map(move |x| *base.add(y * stride + x))).collect();
            CVPixelBufferUnlockBaseAddress(pb, CVPixelBufferLockFlags::ReadOnly);
            v
        }
    }

    #[test]
    fn unscaled_stream_declares_its_size() {
        let (w, h) = (1512, 982);
        let mut e = H264::new(w, h, 4_000_000).unwrap();
        let px = vec![0x80_4020u32; w * h];
        assert_eq!(sps_size(&e.encode(&px, w, true).unwrap().data), Some((w, h)));
    }

    /// Dynamic resolution: 4K frames (in memory and as a captured buffer) encode as a 2/3
    /// picture whose stream declares that size, and the picture is the frame scaled (not a
    /// corner of it).
    #[test]
    fn scaled_frames_encode_at_the_smaller_size() {
        let (sw, sh) = (3840, 2160);
        let (w, h) = crate::vscale::scaled(sw, sh, 2);
        // Four quadrants of different colours, so a crop or a misplaced scale shows.
        let quad = |x: usize, y: usize| [0xff_0000u32, 0x00_ff00, 0x00_00ff, 0xff_ffff][usize::from(x >= sw / 2) + 2 * usize::from(y >= sh / 2)];
        let px: Vec<u32> = (0..sw * sh).map(|i| quad(i % sw, i / sw)).collect();
        let check = |e: &H264, what: &str| {
            let got = staged(e);
            for (x, y) in [(w / 4, h / 4), (3 * w / 4, h / 4), (w / 4, 3 * h / 4), (3 * w / 4, 3 * h / 4)] {
                let want = quad(x * sw / w, y * sh / h);
                let p = got[y * w + x];
                let near = |s: u32| ((p >> s & 255) as i32 - (want >> s & 255) as i32).abs() <= 2;
                assert!(near(0) && near(8) && near(16), "{what} ({x},{y}): {p:06x} vs {want:06x}");
            }
        };
        let mut e = H264::scaled(sw, sh, w, h, 16_000_000).unwrap();
        assert_eq!((e.width, e.height), (2560, 1440));
        let first = e.encode(&px, sw, true).unwrap();
        assert!(first.key);
        assert_eq!(sps_size(&first.data), Some((2560, 1440)));
        check(&e, "cpu");
        let next = e.encode(&px, sw, false).unwrap();
        assert!(!next.key);
        let mut e = H264::scaled(sw, sh, w, h, 16_000_000).unwrap();
        let buffer = surface(sw, sh, quad);
        let first = e.encode_buffer(&buffer, true).unwrap();
        assert!(first.key && !first.data.is_empty());
        assert_eq!(sps_size(&first.data), Some((2560, 1440)));
        check(&e, "transfer");
        assert!(!e.encode_buffer(&buffer, false).unwrap().key);
    }

    /// What VTCompressionSession does with a buffer twice its size (macOS 27: takes it, and
    /// the IDR differs from the cropped picture's): informational, the encoder scales itself.
    /// `cargo test --release session_given_other_sizes -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn session_given_other_sizes() {
        let idr = |pb: &CVPixelBuffer| H264::new(320, 180, 2_000_000).unwrap().encode_ready(pb, true).map(|u| u.data);
        let halves = |w: usize| move |x: usize, _| if x < w / 2 { 0xff_0000 } else { 0x00_00ff };
        let big = idr(&surface(640, 360, halves(640)));
        let cropped = idr(&surface(320, 180, |_, _| 0xff_0000)).unwrap();
        let scaled = idr(&surface(320, 180, halves(320))).unwrap();
        let again = idr(&surface(320, 180, halves(320))).unwrap();
        println!("big {:?} cropped {} scaled {} again-equal {}", big.as_ref().map(|b| (b.len(), b == &cropped, b == &scaled)), cropped.len(), scaled.len(), again == scaled);
    }

    /// Encode time of a scrolling screen-sized picture (ms, p50/p95) and how many frames the
    /// rate control drops, for text and for noise (like video): for comparing settings.
    /// `cargo test --release encode_time -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn encode_time() {
        let (w, h) = (1512, 982);
        let bitrate = crate::server::video_bitrate(w, h) as i64; // as host_mac::video
        let text = |x: usize, y: usize| {
            let glyph = ((x / 7) as u32 ^ (y / 16) as u32 * 31).wrapping_mul(2654435761) >> 29;
            if y % 16 < 11 && x % 7 < 5 && glyph > 1 && (x as u32 ^ y as u32 * 3) % 5 != 0 { 0x20_2020u32 } else { 0xff_ffff }
        };
        let noise = |x: usize, y: usize| (x as u32 * 7 ^ y as u32 * 13).wrapping_mul(2654435761) & 0xff_ffff;
        for (name, f) in [("text", &text as &dyn Fn(usize, usize) -> u32), ("noise", &noise)] {
            let mut e = H264::new(w, h, bitrate).expect("encoder");
            let base: Vec<u32> = (0..w * (h + 400)).map(|i| f(i % w, i / w)).collect();
            let (mut times, mut dropped) = (Vec::new(), 0);
            for n in 0..240 {
                let off = (n * 3 % 400) * w;
                let t = std::time::Instant::now();
                let u = e.encode(&base[off..off + w * h], w, n == 0).unwrap();
                dropped += usize::from(u.data.is_empty());
                if n >= 20 {
                    times.push(t.elapsed().as_secs_f64() * 1000.0);
                }
            }
            times.sort_by(f64::total_cmp);
            let q = |p: f64| times[((times.len() - 1) as f64 * p) as usize];
            println!("{name} {w}x{h} at {bitrate} bit/s: encode p50 {:.2} ms, p95 {:.2} ms; dropped {dropped} of 240", q(0.5), q(0.95));
        }
    }
}
