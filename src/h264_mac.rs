//! Hardware H.264 (VideoToolbox) for RFB encoding 50 ("Open H.264"): one encoder per viewer,
//! fed 0x00RRGGBB frames (BGRA in memory), giving Annex B access units. Real-time, no frame
//! reordering, low-latency rate control; key frames carry SPS and PPS so a decoder can start
//! (or restart) from any of them.
use objc2_core_foundation::{CFBoolean, CFDictionary, CFNumber, CFRetained, CFString, CFType};
use objc2_core_media::{kCMTimeInvalid, CMFormatDescription, CMSampleBuffer, CMTime, CMVideoFormatDescriptionGetH264ParameterSetAtIndex};
use objc2_core_video::{kCVPixelFormatType_32BGRA, CVPixelBuffer, CVPixelBufferCreate, CVPixelBufferGetBaseAddress, CVPixelBufferGetBytesPerRow, CVPixelBufferLockBaseAddress, CVPixelBufferLockFlags, CVPixelBufferUnlockBaseAddress};
use objc2_video_toolbox::*;
use std::ffi::c_void;
use std::ptr::{null_mut, NonNull};
use std::sync::Mutex;

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
    frame: i64,
}

// The session and pixel buffer are used by one viewer thread at a time.
unsafe impl Send for H264 {}

fn set(session: &VTCompressionSession, key: &CFString, value: &CFType) -> bool {
    let s = session as *const VTCompressionSession as *const VTSession;
    unsafe { VTSessionSetProperty(&*s, key, Some(value)) == 0 }
}

impl H264 {
    /// An encoder for width x height frames (even sizes) at about `bitrate` bits a second.
    pub fn new(width: usize, height: usize, bitrate: i64) -> Result<H264, String> {
        let (width, height) = (width & !1, height & !1);
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
            session.prepare_to_encode_frames();
        }
        let mut pb: *mut CVPixelBuffer = null_mut();
        let attrs = unsafe {
            CFDictionary::<CFString, CFType>::from_slices(&[objc2_core_video::kCVPixelBufferIOSurfacePropertiesKey], &[CFDictionary::<CFString, CFType>::empty().as_ref()])
        };
        let r = unsafe { CVPixelBufferCreate(None, width, height, kCVPixelFormatType_32BGRA, Some(attrs.as_opaque()), NonNull::from(&mut pb)) };
        let buffer = NonNull::new(pb).filter(|_| r == 0).ok_or_else(|| format!("CVPixelBufferCreate {r}"))?;
        Ok(H264 { session, out, buffer: unsafe { CFRetained::from_raw(buffer) }, width, height, frame: 0 })
    }

    /// Encode one frame (`px`: rows of `stride` pixels, at least width x height); `key` forces
    /// an IDR with parameter sets. Waits for the encoder, so the result is this frame's.
    pub fn encode(&mut self, px: &[u32], stride: usize, key: bool) -> Result<Unit, String> {
        unsafe {
            let pb = &*self.buffer;
            CVPixelBufferLockBaseAddress(pb, CVPixelBufferLockFlags(0));
            let base = CVPixelBufferGetBaseAddress(pb) as *mut u8;
            let bpr = CVPixelBufferGetBytesPerRow(pb);
            for y in 0..self.height {
                let src = &px[y * stride..y * stride + self.width];
                std::ptr::copy_nonoverlapping(src.as_ptr() as *const u8, base.add(y * bpr), self.width * 4);
            }
            CVPixelBufferUnlockBaseAddress(pb, CVPixelBufferLockFlags(0));
        }
        self.encode_buffer(&self.buffer.clone(), key)
    }

    /// Encode a ready pixel buffer (e.g. a captured IOSurface) of this encoder's size.
    pub fn encode_buffer(&mut self, pb: &CVPixelBuffer, key: bool) -> Result<Unit, String> {
        let pts = unsafe { CMTime::new(self.frame, 60) };
        self.frame += 1;
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
mod tests {
    use super::*;

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
}
