//! H.264 ("Open H.264", RFB encoding 50) for the Windows viewer through Media Foundation's
//! H.264 decoder MFT, in software: Annex B in, one access unit per sample, NV12 out, converted
//! into the framebuffer (yuv.rs). Itda's own sharing server sends moving screens this way at a
//! fraction of ZRLE's size. Any failure is returned as an error, and the viewer goes back to
//! ZRLE (net.rs) instead of ending the session.
use crate::yuv::{self, Nv12, Yuv};
use std::sync::atomic::{AtomicBool, Ordering::Relaxed};
use std::sync::OnceLock;
use std::time::{Duration, Instant};
use windows::core::Interface;
use windows::Win32::Media::MediaFoundation::*;
use windows::Win32::System::Com::{CoCreateInstance, CLSCTX_INPROC_SERVER};

/// Inputs in a row that may give no picture before the decoder counts as broken: in low latency
/// mode every access unit the server sends is a picture, so a decoder that holds them back would
/// freeze the screen.
const STARVED: u32 = 30;

/// Set when decoding failed in this process: later connections ask for ZRLE only.
static OFF: AtomicBool = AtomicBool::new(false);

/// Whether to ask servers for H.264: a decoder can be made here, it hasn't failed before in this
/// process, and ITDA_NO_H264=1 isn't set. The probe runs once, on a thread of its own so its COM
/// apartment can't clash with the caller's (the UI thread's is single-threaded for OLE).
pub fn available() -> bool {
    static PROBE: OnceLock<bool> = OnceLock::new();
    !OFF.load(Relaxed)
        && *PROBE.get_or_init(|| {
            if std::env::var("ITDA_NO_H264").as_deref() == Ok("1") {
                crate::log!("h264: ITDA_NO_H264=1, asking for ZRLE only");
                return false;
            }
            let ok = std::thread::spawn(|| match Decoder::new(1920, 1080) {
                Ok(_) => true,
                Err(e) => {
                    crate::log!("h264: no decoder here ({e}); asking for ZRLE only");
                    false
                }
            })
            .join()
            .unwrap_or(false);
            if ok {
                crate::log!("h264: Media Foundation decoder available");
            }
            ok
        })
}

/// Stop asking for H.264 in this process (after a decoding failure).
pub fn disable() {
    OFF.store(true, Relaxed);
}

pub struct Decoder {
    t: IMFTransform,
    /// The rectangle size it was made for: a reset to another size makes a new decoder.
    pub made: (usize, usize),
    /// Output pictures: NV12 of `coded` (its height padded to whole macroblocks), of which
    /// `shown` from the top left is the picture.
    coded: (usize, usize),
    shown: (usize, usize),
    /// Row pitch when a buffer doesn't tell its own (MF_MT_DEFAULT_STRIDE, else the width).
    stride: usize,
    yuv: Yuv,
    /// The MFT allocates output samples itself (else `out` is ours, reused for every picture).
    provides: bool,
    out_size: u32,
    /// What the decoder said of its output stream (for the buffer we give it, and errors).
    align: u32,
    info_flags: u32,
    out: Option<IMFSample>,
    /// Our other output sample, for the next picture.
    spare: Option<IMFSample>,
    time: i64,
    /// Inputs since the last picture.
    idle: u32,
    frames: u64,
    busy: Duration,
}

fn open(w: usize, h: usize) -> windows::core::Result<IMFTransform> {
    unsafe {
        let t: IMFTransform = CoCreateInstance(&CLSID_MSH264DecoderMFT, None, CLSCTX_INPROC_SERVER)?;
        // Without it the decoder keeps pictures back for reordering the server never does.
        match t.GetAttributes() {
            Ok(a) => {
                if let Err(e) = a.SetUINT32(&MF_LOW_LATENCY, 1) {
                    crate::log!("h264: decoder refused low latency mode: {e}");
                }
            }
            Err(e) => crate::log!("h264: decoder has no attributes for low latency: {e}"),
        }
        let inp = MFCreateMediaType()?;
        inp.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Video)?;
        inp.SetGUID(&MF_MT_SUBTYPE, &MFVideoFormat_H264)?;
        inp.SetUINT64(&MF_MT_FRAME_SIZE, ((w as u64) << 32) | h as u64)?;
        inp.SetUINT32(&MF_MT_INTERLACE_MODE, MFVideoInterlace_Progressive.0 as u32)?;
        t.SetInputType(0, &inp, 0)?;
        Ok(t)
    }
}

impl Decoder {
    pub fn new(w: usize, h: usize) -> Result<Decoder, String> {
        if !crate::h264_win::mf_ready() {
            return Err("Media Foundation unavailable".into());
        }
        let t = open(w.max(16), h.max(16)).map_err(|e| format!("H.264 decoder: {e}"))?;
        let mut d = Decoder {
            t,
            made: (w, h),
            coded: (0, 0),
            shown: (0, 0),
            stride: 0,
            yuv: Yuv::new(true, false),
            provides: false,
            out_size: 0,
            align: 0,
            info_flags: 0,
            out: None,
            spare: None,
            time: 0,
            idle: 0,
            frames: 0,
            busy: Duration::ZERO,
        };
        d.configure().map_err(|e| format!("H.264 decoder output: {e}"))?;
        unsafe {
            d.t.ProcessMessage(MFT_MESSAGE_NOTIFY_BEGIN_STREAMING, 0).map_err(|e| format!("H.264 decoder start: {e}"))?;
            d.t.ProcessMessage(MFT_MESSAGE_NOTIFY_START_OF_STREAM, 0).map_err(|e| format!("H.264 decoder start: {e}"))?;
        }
        Ok(d)
    }

    /// Choose NV12 output and read its layout: at creation and whenever the stream changes
    /// (a new SPS: the real size, cropping and colours are only known then).
    fn configure(&mut self) -> windows::core::Result<()> {
        unsafe {
            let mut chosen = None;
            for i in 0..64 {
                let mt = self.t.GetOutputAvailableType(0, i)?;
                if mt.GetGUID(&MF_MT_SUBTYPE)? == MFVideoFormat_NV12 {
                    chosen = Some(mt);
                    break;
                }
            }
            let Some(mt) = chosen else { return Err(windows::core::Error::new(MF_E_INVALIDMEDIATYPE, "no NV12 output")) };
            self.t.SetOutputType(0, &mt, 0)?;
            let mt = self.t.GetOutputCurrentType(0).unwrap_or(mt);
            let size = mt.GetUINT64(&MF_MT_FRAME_SIZE)?;
            let coded = ((size >> 32) as usize, (size & 0xffff_ffff) as usize);
            // MFVideoArea: two MFOffsets (u16 fraction, i16 value) and a SIZE (two i32).
            let mut area = [0u8; 16];
            let shown = match mt.GetBlob(&MF_MT_MINIMUM_DISPLAY_APERTURE, &mut area, None) {
                Ok(()) => {
                    let at = |i: usize| i32::from_le_bytes([area[i], area[i + 1], area[i + 2], area[i + 3]]);
                    let (ox, oy) = (i16::from_le_bytes([area[2], area[3]]), i16::from_le_bytes([area[6], area[7]]));
                    if ox != 0 || oy != 0 {
                        crate::log!("h264: display aperture offset {ox},{oy} ignored");
                    }
                    (at(8).clamp(0, coded.0 as i32) as usize, at(12).clamp(0, coded.1 as i32) as usize)
                }
                Err(_) => coded,
            };
            let stride = mt.GetUINT32(&MF_MT_DEFAULT_STRIDE).map(|s| (s as i32).unsigned_abs() as usize).unwrap_or(0);
            // Unsigned streams: VideoToolbox and Media Foundation tag what they encode; BT.709
            // limited is what HD encoders produce when they don't say.
            let bt709 = mt.GetUINT32(&MF_MT_YUV_MATRIX).map_or(true, |m| m != MFVideoTransferMatrix_BT601.0 as u32);
            let full = mt.GetUINT32(&MF_MT_VIDEO_NOMINAL_RANGE).is_ok_and(|r| r == MFNominalRange_0_255.0 as u32);
            let info = self.t.GetOutputStreamInfo(0)?;
            self.provides = info.dwFlags & (MFT_OUTPUT_STREAM_PROVIDES_SAMPLES.0 as u32 | MFT_OUTPUT_STREAM_CAN_PROVIDE_SAMPLES.0 as u32) != 0;
            // Pictures are whole macroblocks: a 360-line stream decodes into 368 lines, which
            // the first (pre-SPS) type doesn't know yet. Room for that, whatever it says.
            let (aw, ah) = ((coded.0.max(stride) + 15) & !15, (coded.1 + 15) & !15);
            self.out_size = info.cbSize.max((aw * ah * 3 / 2) as u32);
            self.align = info.cbAlignment;
            self.info_flags = info.dwFlags;
            self.out = None;
            self.spare = None;
            (self.coded, self.shown, self.stride, self.yuv) = (coded, shown, stride.max(coded.0), Yuv::new(bt709, full));
            crate::log!(
                "h264: output NV12 {}x{} shown {}x{} stride {} {} {} range{}",
                coded.0, coded.1, shown.0, shown.1, self.stride,
                if bt709 { "BT.709" } else { "BT.601" },
                if full { "full" } else { "limited" },
                if self.provides { ", decoder's samples" } else { "" },
            );
        }
        Ok(())
    }

    /// An output sample for the decoder to fill: the spare one (emptied), or a new one. Never
    /// the one holding the last picture: the decoder refuses a sample it filled before
    /// (CopyDecodedFrame fails with E_FAIL), so two take turns.
    fn out_sample(&mut self) -> windows::core::Result<IMFSample> {
        if let Some(s) = self.spare.take() {
            unsafe { s.GetBufferByIndex(0)?.SetCurrentLength(0)? };
            return Ok(s);
        }
        unsafe {
            let s = MFCreateSample()?;
            let buf = if self.align > 1 { MFCreateAlignedMemoryBuffer(self.out_size, self.align - 1)? } else { MFCreateMemoryBuffer(self.out_size)? };
            s.AddBuffer(&buf)?;
            Ok(s)
        }
    }

    /// Take every picture the decoder has ready (the last one stays in `out`).
    fn drain(&mut self) -> windows::core::Result<bool> {
        let mut got = false;
        // (Bounded: a decoder that keeps reporting stream changes is broken.)
        for _ in 0..64 {
            let given = if self.provides { None } else { Some(self.out_sample()?) };
            let ours = given.clone();
            let mut db = [MFT_OUTPUT_DATA_BUFFER { dwStreamID: 0, pSample: std::mem::ManuallyDrop::new(given), dwStatus: 0, pEvents: std::mem::ManuallyDrop::new(None) }];
            let mut status = 0u32;
            let r = unsafe { self.t.ProcessOutput(0, &mut db, &mut status) };
            let sample = unsafe { std::mem::ManuallyDrop::take(&mut db[0].pSample) };
            drop(unsafe { std::mem::ManuallyDrop::take(&mut db[0].pEvents) });
            match r {
                Err(e) if e.code() == MF_E_TRANSFORM_NEED_MORE_INPUT => {
                    self.spare = ours;
                    return Ok(got);
                }
                // (A new layout: new samples of its size.)
                Err(e) if e.code() == MF_E_TRANSFORM_STREAM_CHANGE => self.configure()?,
                Err(e) => {
                    let why = format!("{} (coded {}x{} stride {}, buffer {} bytes, stream flags {:#x} alignment {}{})", e.message(), self.coded.0, self.coded.1, self.stride, self.out_size, self.info_flags, self.align, if self.provides { ", decoder's samples" } else { "" });
                    return Err(windows::core::Error::new(e.code(), why));
                }
                Ok(()) => {
                    // The new picture's sample is shown; the one before becomes the spare.
                    let filled = if self.provides { sample } else { ours };
                    if filled.is_some() {
                        let before = std::mem::replace(&mut self.out, filled);
                        if !self.provides {
                            self.spare = before;
                        }
                    }
                    got |= self.out.is_some();
                }
            }
        }
        Err(windows::core::Error::new(MF_E_UNEXPECTED, "decoder output never settles"))
    }

    fn feed(&mut self, au: &[u8]) -> windows::core::Result<bool> {
        unsafe {
            let buf = MFCreateMemoryBuffer(au.len() as u32)?;
            let mut p = std::ptr::null_mut();
            buf.Lock(&mut p, None, None)?;
            std::ptr::copy_nonoverlapping(au.as_ptr(), p, au.len());
            buf.Unlock()?;
            buf.SetCurrentLength(au.len() as u32)?;
            let sample = MFCreateSample()?;
            sample.AddBuffer(&buf)?;
            // Timestamps only order the samples (the decoder wants them); 60 per second.
            sample.SetSampleTime(self.time * 166_667)?;
            sample.SetSampleDuration(166_667)?;
            self.time += 1;
            match self.t.ProcessInput(0, &sample, 0) {
                // Full: take what is ready, then it accepts the input.
                Err(e) if e.code() == MF_E_NOTACCEPTING => {
                    let got = self.drain()?;
                    self.t.ProcessInput(0, &sample, 0)?;
                    Ok(self.drain()? || got)
                }
                Err(e) => Err(e),
                Ok(()) => self.drain(),
            }
        }
    }

    /// Decode one rectangle's data (`reset`: the server started a new stream, which begins
    /// with an IDR and its parameter sets) and hand the newest picture to `draw`, if any.
    pub fn decode(&mut self, data: &[u8], reset: bool, draw: impl FnOnce(&Nv12, &Yuv)) -> Result<(), String> {
        let start = Instant::now();
        if reset {
            unsafe { self.t.ProcessMessage(MFT_MESSAGE_COMMAND_FLUSH, 0) }.map_err(|e| format!("flush: {e}"))?;
            self.idle = 0;
            if !yuv::has_nal(data, 5) {
                crate::log!("h264: reset without an IDR ({} bytes)", data.len());
            }
        }
        let mut got = false;
        for au in yuv::access_units(data) {
            got |= self.feed(au).map_err(|e| format!("decode: {e}"))?;
        }
        if !got {
            self.idle += 1;
            if self.idle >= STARVED {
                return Err(format!("no picture after {STARVED} inputs"));
            }
            return Ok(());
        }
        self.idle = 0;
        self.picture(draw).map_err(|e| format!("output buffer: {e}"))?;
        self.frames += 1;
        self.busy += start.elapsed();
        if self.frames == 1 || self.frames % 600 == 0 {
            crate::log!("h264: {} pictures, {:.1} ms each on average", self.frames, self.busy.as_secs_f64() * 1000.0 / self.frames as f64);
        }
        Ok(())
    }

    /// Lock the last picture's buffer and show it to `draw`.
    fn picture(&mut self, draw: impl FnOnce(&Nv12, &Yuv)) -> windows::core::Result<()> {
        let Some(sample) = self.out.clone() else { return Ok(()) };
        let (cw, ch) = self.coded;
        unsafe {
            let buf = sample.GetBufferByIndex(0)?;
            // A 2D buffer knows its pitch; locked as a plain buffer it would be copied to the
            // minimum one first.
            if let Ok(b2) = buf.cast::<IMF2DBuffer>() {
                let (mut p, mut pitch) = (std::ptr::null_mut(), 0i32);
                b2.Lock2D(&mut p, &mut pitch)?;
                let stride = pitch as usize;
                if pitch > 0 && stride >= cw && !p.is_null() {
                    let all = std::slice::from_raw_parts(p, stride * ch * 3 / 2);
                    self.show(all, stride, draw);
                } else {
                    crate::log!("h264: unusable pitch {pitch} for {cw}x{ch}");
                }
                b2.Unlock2D()?;
                return Ok(());
            }
            let (mut p, mut len) = (std::ptr::null_mut(), 0u32);
            buf.Lock(&mut p, None, Some(&mut len))?;
            if (len as usize) < self.stride * ch * 3 / 2 || p.is_null() {
                crate::log!("h264: picture buffer of {len} bytes for {cw}x{ch} stride {}", self.stride);
            } else {
                self.show(std::slice::from_raw_parts(p, len as usize), self.stride, draw);
            }
            buf.Unlock()?;
        }
        Ok(())
    }

    fn show(&self, all: &[u8], stride: usize, draw: impl FnOnce(&Nv12, &Yuv)) {
        let ch = self.coded.1;
        let (y, uv) = all.split_at(stride * ch);
        draw(&Nv12 { y, uv, stride, w: self.shown.0, h: self.shown.1 }, &self.yuv);
    }
}

impl Drop for Decoder {
    fn drop(&mut self) {
        unsafe {
            let _ = self.t.ProcessMessage(MFT_MESSAGE_NOTIFY_END_OF_STREAM, 0);
            let _ = self.t.ProcessMessage(MFT_MESSAGE_NOTIFY_END_STREAMING, 0);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Where Media Foundation has both H.264 MFTs (a desktop Windows): what the encoder makes
    /// decodes to the same picture, give or take compression.
    #[test]
    fn decodes_what_the_encoder_makes() {
        use crate::server::VideoEncode;
        let (w, h) = (640, 360);
        let mut e = match crate::h264_win::H264::new(w, h, 8_000_000) {
            Ok(e) => e,
            Err(why) => return eprintln!("skipped: no Media Foundation H.264 encoder here ({why})"),
        };
        let mut d = match Decoder::new(w, h) {
            Ok(d) => d,
            Err(why) => return eprintln!("skipped: no Media Foundation H.264 decoder here ({why})"),
        };
        // Broad grey bands, which survive compression closely whichever matrix the encoder used
        // (it converts with BT.601 and may not say so).
        let px: Vec<u32> = (0..w * h).map(|i| [0x000000, 0x404040, 0xa0a0a0, 0xffffff][(i % w) * 4 / w]).collect();
        let mut out = vec![0u32; w * h];
        let mut pictures = 0;
        for i in 0..3 {
            let data = e.encode(&px, w, i == 0).unwrap();
            d.decode(&data, i == 0, |pic, yuv| {
                assert_eq!((pic.w, pic.h), (w, h));
                assert!(yuv.convert(pic, &mut out, w));
                pictures += 1;
            })
            .unwrap();
        }
        assert!(pictures >= 2, "low latency: a picture per input ({pictures})");
        for (x, want) in [(40, 0x000000u32), (200, 0x404040), (360, 0xa0a0a0), (600, 0xffffff)] {
            let got = out[180 * w + x];
            let close = (0..3).all(|i| (((got >> (i * 8)) & 255) as i32 - ((want >> (i * 8)) & 255) as i32).abs() <= 16);
            assert!(close, "at {x}: {got:06x} vs {want:06x}");
        }
    }
}
