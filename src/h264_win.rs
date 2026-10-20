//! H.264 for Share this PC (RFB encoding 50) through Media Foundation's H.264 encoder MFT,
//! one per viewer: BGRA frames converted to NV12, low-latency mode, Annex B out. A forced
//! key frame (join, reset, full request) starts a fresh encoder, whose first frame is an IDR
//! with SPS/PPS. Any failure returns None/Err and the server keeps using ZRLE.
use crate::server::{Video, VideoEncode};
use std::io;
use std::sync::OnceLock;
use windows::Win32::Media::MediaFoundation::*;
use windows::Win32::System::Com::{CoCreateInstance, CoInitializeEx, CoUninitialize, CLSCTX_INPROC_SERVER, COINIT_MULTITHREADED};

const FPS: u64 = 60;
const TICKS: i64 = 10_000_000 / FPS as i64; // 100 ns units per frame

struct Mft {
    t: IMFTransform,
    provides_samples: bool,
    out_size: u32,
}

pub struct H264 {
    mft: Mft,
    w: usize,
    h: usize,
    bitrate: u32,
    frame: i64,
    /// This encoder has given output: a forced key must start a fresh one (until then, its
    /// first output is an IDR anyway).
    produced: bool,
    nv12: Vec<u8>,
}

// Used by one viewer's pump thread at a time.
unsafe impl Send for H264 {}

fn startup() -> bool {
    static OK: OnceLock<bool> = OnceLock::new();
    *OK.get_or_init(|| unsafe { MFStartup(MF_VERSION, MFSTARTUP_LITE).is_ok() })
}

/// COM on this (viewer) thread, once, released when the thread ends — after its encoders,
/// which the pump drops before returning.
struct Com(bool);
impl Drop for Com {
    fn drop(&mut self) {
        if self.0 {
            unsafe { CoUninitialize() };
        }
    }
}
thread_local!(static COM: Com = Com(unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) }.is_ok()));

fn open(w: usize, h: usize, bitrate: u32) -> windows::core::Result<Mft> {
    COM.with(|_| {});
    unsafe {
        let t: IMFTransform = CoCreateInstance(&CLSID_MSH264EncoderMFT, None, CLSCTX_INPROC_SERVER)?;
        if let Ok(a) = t.GetAttributes() {
            let _ = a.SetUINT32(&MF_LOW_LATENCY, 1);
        }
        let size = ((w as u64) << 32) | h as u64;
        let rate = (FPS << 32) | 1;
        let out = MFCreateMediaType()?;
        out.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Video)?;
        out.SetGUID(&MF_MT_SUBTYPE, &MFVideoFormat_H264)?;
        out.SetUINT32(&MF_MT_AVG_BITRATE, bitrate)?;
        out.SetUINT64(&MF_MT_FRAME_SIZE, size)?;
        out.SetUINT64(&MF_MT_FRAME_RATE, rate)?;
        out.SetUINT32(&MF_MT_INTERLACE_MODE, MFVideoInterlace_Progressive.0 as u32)?;
        out.SetUINT32(&MF_MT_MPEG2_PROFILE, eAVEncH264VProfile_Main.0 as u32)?;
        t.SetOutputType(0, &out, 0)?;
        let inp = MFCreateMediaType()?;
        inp.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Video)?;
        inp.SetGUID(&MF_MT_SUBTYPE, &MFVideoFormat_NV12)?;
        inp.SetUINT64(&MF_MT_FRAME_SIZE, size)?;
        inp.SetUINT64(&MF_MT_FRAME_RATE, rate)?;
        inp.SetUINT32(&MF_MT_INTERLACE_MODE, MFVideoInterlace_Progressive.0 as u32)?;
        t.SetInputType(0, &inp, 0)?;
        let info = t.GetOutputStreamInfo(0)?;
        t.ProcessMessage(MFT_MESSAGE_NOTIFY_BEGIN_STREAMING, 0)?;
        t.ProcessMessage(MFT_MESSAGE_NOTIFY_START_OF_STREAM, 0)?;
        Ok(Mft {
            t,
            provides_samples: info.dwFlags & (MFT_OUTPUT_STREAM_PROVIDES_SAMPLES.0 as u32 | MFT_OUTPUT_STREAM_CAN_PROVIDE_SAMPLES.0 as u32) != 0,
            out_size: info.cbSize.max((w * h * 2) as u32),
        })
    }
}

impl H264 {
    pub fn new(w: usize, h: usize, bitrate: u32) -> Result<H264, String> {
        let (w, h) = (w & !1, h & !1);
        if w < 16 || h < 16 || !startup() {
            return Err("Media Foundation unavailable".into());
        }
        let mft = open(w, h, bitrate).map_err(|e| format!("H.264 encoder: {e}"))?;
        Ok(H264 { mft, w, h, bitrate, frame: 0, produced: false, nv12: vec![0; w * h * 3 / 2] })
    }

    /// BGRA (0x00RRGGBB) to NV12, BT.601 limited range.
    fn convert(&mut self, px: &[u32], stride: usize) {
        let (w, h) = (self.w, self.h);
        let (y_plane, uv) = self.nv12.split_at_mut(w * h);
        for y in 0..h {
            let row = &px[y * stride..y * stride + w];
            for (d, p) in y_plane[y * w..y * w + w].iter_mut().zip(row) {
                let (r, g, b) = (((p >> 16) & 255) as i32, ((p >> 8) & 255) as i32, (p & 255) as i32);
                *d = (((66 * r + 129 * g + 25 * b + 128) >> 8) + 16) as u8;
            }
        }
        for y in (0..h).step_by(2) {
            for x in (0..w).step_by(2) {
                let mut s = [0i32; 3];
                for (dx, dy) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
                    let p = px[(y + dy) * stride + x + dx];
                    s[0] += ((p >> 16) & 255) as i32;
                    s[1] += ((p >> 8) & 255) as i32;
                    s[2] += (p & 255) as i32;
                }
                let (r, g, b) = (s[0] / 4, s[1] / 4, s[2] / 4);
                let o = (y / 2) * w + x;
                uv[o] = (((-38 * r - 74 * g + 112 * b + 128) >> 8) + 128) as u8;
                uv[o + 1] = (((112 * r - 94 * g - 18 * b + 128) >> 8) + 128) as u8;
            }
        }
    }

    fn encode_frame(&mut self, px: &[u32], stride: usize, key: bool) -> windows::core::Result<Vec<u8>> {
        if key && self.produced {
            self.mft = open(self.w, self.h, self.bitrate)?;
            (self.frame, self.produced) = (0, false);
        }
        self.convert(px, stride);
        unsafe {
            let buf = MFCreateMemoryBuffer(self.nv12.len() as u32)?;
            let mut p = std::ptr::null_mut();
            buf.Lock(&mut p, None, None)?;
            std::ptr::copy_nonoverlapping(self.nv12.as_ptr(), p, self.nv12.len());
            buf.Unlock()?;
            buf.SetCurrentLength(self.nv12.len() as u32)?;
            let sample = MFCreateSample()?;
            sample.AddBuffer(&buf)?;
            sample.SetSampleTime(self.frame * TICKS)?;
            sample.SetSampleDuration(TICKS)?;
            self.frame += 1;
            self.mft.t.ProcessInput(0, &sample, 0)?;
            let mut out = Vec::new();
            loop {
                let given = if self.mft.provides_samples {
                    None
                } else {
                    let s = MFCreateSample()?;
                    s.AddBuffer(&MFCreateMemoryBuffer(self.mft.out_size)?)?;
                    Some(s)
                };
                let mut db = [MFT_OUTPUT_DATA_BUFFER { dwStreamID: 0, pSample: std::mem::ManuallyDrop::new(given), dwStatus: 0, pEvents: std::mem::ManuallyDrop::new(None) }];
                let mut status = 0u32;
                let r = self.mft.t.ProcessOutput(0, &mut db, &mut status);
                let sample = std::mem::ManuallyDrop::take(&mut db[0].pSample);
                drop(std::mem::ManuallyDrop::take(&mut db[0].pEvents));
                match r {
                    Err(e) if e.code() == MF_E_TRANSFORM_NEED_MORE_INPUT => break,
                    Err(e) => return Err(e),
                    Ok(()) => {}
                }
                if let Some(s) = sample {
                    let b = s.ConvertToContiguousBuffer()?;
                    let (mut p, mut len) = (std::ptr::null_mut(), 0u32);
                    b.Lock(&mut p, None, Some(&mut len))?;
                    out.extend_from_slice(std::slice::from_raw_parts(p, len as usize));
                    b.Unlock()?;
                }
            }
            // An IDR without parameter sets in-band: take them from the output type
            // (MF_MT_MPEG_SEQUENCE_HEADER), so a decoder can start here.
            let nal = |t: u8| out.windows(5).any(|w| w[..4] == [0, 0, 0, 1] && w[4] & 31 == t);
            if nal(5) && !nal(7) {
                if let Ok(mt) = self.mft.t.GetOutputCurrentType(0) {
                    if let Ok(n) = mt.GetBlobSize(&MF_MT_MPEG_SEQUENCE_HEADER) {
                        let mut seq = vec![0u8; n as usize];
                        if mt.GetBlob(&MF_MT_MPEG_SEQUENCE_HEADER, &mut seq, None).is_ok() {
                            out.splice(0..0, seq);
                        }
                    }
                }
            }
            self.produced |= !out.is_empty();
            Ok(out)
        }
    }
}

impl VideoEncode for H264 {
    fn size(&self) -> (usize, usize) {
        (self.w, self.h)
    }
    fn encode(&mut self, px: &[u32], stride: usize, key: bool) -> io::Result<Vec<u8>> {
        self.encode_frame(px, stride, key).map_err(io::Error::other)
    }
}

/// An encoder for a w×h screen, or None (no Media Foundation H.264 encoder here).
pub fn video(w: usize, h: usize) -> Option<Video> {
    let bitrate = ((w * h * 6) as u32).clamp(4_000_000, 30_000_000);
    match H264::new(w, h, bitrate) {
        Ok(e) => Some(Box::new(e)),
        Err(e) => {
            crate::log!("share: no H.264 encoder ({e}); viewers get ZRLE");
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Where Media Foundation's encoder exists (a desktop Windows; not every CI image), its
    /// first frame is an IDR with SPS/PPS and a nearly unchanged frame is small.
    #[test]
    fn encodes_annex_b() {
        let mut e = match H264::new(640, 360, 4_000_000) {
            Ok(e) => e,
            Err(why) => return eprintln!("skipped: no Media Foundation H.264 encoder here ({why})"),
        };
        let mut px: Vec<u32> = (0..640 * 360).map(|i| (i as u32).wrapping_mul(2654435761) & 0xff_ffff).collect();
        let first = e.encode(&px, 640, true).unwrap();
        let types: Vec<u8> = first.windows(4).enumerate().filter(|(_, w)| *w == [0, 0, 0, 1]).map(|(i, _)| first[i + 4] & 31).collect();
        assert!(types.contains(&7) && types.contains(&8) && types.contains(&5), "{types:?}");
        px[1000] ^= 0xffffff;
        let next = e.encode(&px, 640, false).unwrap();
        assert!(next.len() < first.len(), "{} vs {}", next.len(), first.len());
    }
}
