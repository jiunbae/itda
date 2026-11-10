//! This PC's sound for viewers of "Share this PC" (audio.rs): a WASAPI loopback of the default
//! output device — what the speakers play, never the microphone — on a thread of its own,
//! polled every 10 ms (event-driven loopback needs Windows 10).
//!
//! It follows the default device: the endpoint is checked every second and the capture
//! reopened on another one; a device that goes away (headphones unplugged) or none at all is
//! retried every second. Loopback delivers nothing while nothing plays, and buffers marked
//! silent are skipped, so a quiet PC sends nothing.

use crate::audio::{Capture, Sink};
use std::ffi::c_void;
use std::sync::atomic::{AtomicBool, Ordering::Relaxed};
use std::sync::Arc;
use std::time::{Duration, Instant};
use windows::core::{GUID, HRESULT};
use windows::Win32::Media::Audio::*;
use windows::Win32::System::Com::{CoCreateInstance, CoInitializeEx, CoTaskMemFree, CoUninitialize, CLSCTX_ALL, COINIT_MULTITHREADED};

/// Shared-mode buffer (100 ns units): room for 200 ms between polls.
const BUFFER: i64 = 2_000_000;
const POLL: Duration = Duration::from_millis(10);
const CHECK: Duration = Duration::from_secs(1);
const WAVE_FORMAT_IEEE_FLOAT: u16 = 3;
const WAVE_FORMAT_EXTENSIBLE: u16 = 0xfffe;
const SUBTYPE_PCM: GUID = GUID::from_u128(0x00000001_0000_0010_8000_00aa00389b71);
const SUBTYPE_FLOAT: GUID = GUID::from_u128(0x00000003_0000_0010_8000_00aa00389b71);

pub(crate) fn audio_supported() -> bool {
    true
}

/// Start the loopback on a thread of its own; dropping the result stops it (within POLL).
pub(crate) fn audio_capture(sink: Sink) -> Option<Capture> {
    let stop = Arc::new(AtomicBool::new(false));
    let s = stop.clone();
    std::thread::Builder::new().name("itda-audio".into()).spawn(move || run(&s, sink)).ok()?;
    Some(Box::new(Stop(stop)))
}

struct Stop(Arc<AtomicBool>);
impl Drop for Stop {
    fn drop(&mut self) {
        self.0.store(true, Relaxed);
    }
}

fn run(stop: &AtomicBool, mut sink: Sink) {
    let com = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) }.is_ok();
    let mut said = String::new();
    while !stop.load(Relaxed) {
        if let Err(e) = unsafe { session(stop, &mut sink) } {
            // (Once per kind of failure, not every second while there's no device.)
            let why = e.message();
            if why != said {
                crate::log!("share: sound capture: {why} ({:#x})", e.code().0);
                said = why;
            }
            for _ in 0..20 {
                if stop.load(Relaxed) {
                    break;
                }
                std::thread::sleep(Duration::from_millis(50));
            }
        }
    }
    if com {
        unsafe { CoUninitialize() };
    }
}

/// The mix format, freed with the COM allocator.
struct Mix(*mut WAVEFORMATEX);
impl Drop for Mix {
    fn drop(&mut self) {
        unsafe { CoTaskMemFree(Some(self.0 as *const c_void)) };
    }
}

fn device_id(d: &IMMDevice) -> Option<String> {
    unsafe {
        let p = d.GetId().ok()?;
        let id = p.to_string().ok();
        CoTaskMemFree(Some(p.0 as *const c_void));
        id
    }
}

/// Capture from the default device until it changes (Ok) or fails (Err).
unsafe fn session(stop: &AtomicBool, sink: &mut Sink) -> windows::core::Result<()> {
    let devices: IMMDeviceEnumerator = CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL)?;
    let device = devices.GetDefaultAudioEndpoint(eRender, eConsole)?;
    let id = device_id(&device);
    let client: IAudioClient = device.Activate(CLSCTX_ALL, None)?;
    let mix = Mix(client.GetMixFormat()?);
    let f: WAVEFORMATEX = std::ptr::read_unaligned(mix.0);
    let (tag, channels, rate, align, bits) = (f.wFormatTag, f.nChannels as usize, f.nSamplesPerSec, f.nBlockAlign as usize, f.wBitsPerSample);
    let sub = if tag == WAVE_FORMAT_EXTENSIBLE && f.cbSize >= 22 { Some(std::ptr::read_unaligned(mix.0 as *const WAVEFORMATEXTENSIBLE).SubFormat) } else { None };
    let unsupported = || windows::core::Error::new(HRESULT(0x8889_0008u32 as i32), format!("unsupported mix format (tag {tag:#x}, {bits} bits)")); // AUDCLNT_E_UNSUPPORTED_FORMAT
    let kind = sample_kind(tag, bits, sub).ok_or_else(unsupported)?;
    if channels == 0 || align < channels * kind.bytes() {
        return Err(unsupported());
    }
    client.Initialize(AUDCLNT_SHAREMODE_SHARED, AUDCLNT_STREAMFLAGS_LOOPBACK, BUFFER, 0, mix.0, None)?;
    let capture: IAudioCaptureClient = client.GetService()?;
    client.Start()?;
    let r = (|| -> windows::core::Result<()> {
        let mut checked = Instant::now();
        let mut buf = Vec::new();
        while !stop.load(Relaxed) {
            std::thread::sleep(POLL);
            while capture.GetNextPacketSize()? > 0 {
                let (mut data, mut frames, mut flags) = (std::ptr::null_mut(), 0u32, 0u32);
                capture.GetBuffer(&mut data, &mut frames, &mut flags, None, None)?;
                let sound = flags & AUDCLNT_BUFFERFLAGS_SILENT.0 as u32 == 0 && !data.is_null() && frames > 0;
                if sound {
                    to_f32(kind, std::slice::from_raw_parts(data, frames as usize * align), align, channels, &mut buf);
                }
                capture.ReleaseBuffer(frames)?;
                if sound {
                    sink(&buf, rate, channels);
                }
            }
            if checked.elapsed() >= CHECK {
                checked = Instant::now();
                let now = devices.GetDefaultAudioEndpoint(eRender, eConsole).ok().and_then(|d| device_id(&d));
                if now != id {
                    crate::log!("share: the default sound output changed: capturing the new one");
                    return Ok(());
                }
            }
        }
        Ok(())
    })();
    let _ = client.Stop();
    r
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Kind {
    F32,
    I16,
    I24,
    I32,
}

impl Kind {
    fn bytes(self) -> usize {
        match self {
            Kind::I16 => 2,
            Kind::I24 => 3,
            _ => 4,
        }
    }
}

/// What a mix format's samples are (shared mode is float in practice; PCM just in case).
fn sample_kind(tag: u16, bits: u16, sub: Option<GUID>) -> Option<Kind> {
    let float = tag == WAVE_FORMAT_IEEE_FLOAT || sub == Some(SUBTYPE_FLOAT);
    let pcm = tag == WAVE_FORMAT_PCM as u16 || sub == Some(SUBTYPE_PCM);
    match (float, pcm, bits) {
        (true, _, 32) => Some(Kind::F32),
        (_, true, 16) => Some(Kind::I16),
        (_, true, 24) => Some(Kind::I24),
        (_, true, 32) => Some(Kind::I32),
        _ => None,
    }
}

/// Frames of `align` bytes into interleaved floats (`channels` per frame).
fn to_f32(kind: Kind, data: &[u8], align: usize, channels: usize, out: &mut Vec<f32>) {
    out.clear();
    let n = kind.bytes();
    for frame in data.chunks_exact(align) {
        for s in frame[..channels * n].chunks_exact(n) {
            out.push(match kind {
                Kind::F32 => f32::from_le_bytes([s[0], s[1], s[2], s[3]]),
                Kind::I16 => i16::from_le_bytes([s[0], s[1]]) as f32 / 32768.0,
                Kind::I24 => i32::from_le_bytes([0, s[0], s[1], s[2]]) as f32 / 2147483648.0,
                Kind::I32 => i32::from_le_bytes([s[0], s[1], s[2], s[3]]) as f32 / 2147483648.0,
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mix_formats() {
        assert_eq!(sample_kind(WAVE_FORMAT_EXTENSIBLE, 32, Some(SUBTYPE_FLOAT)), Some(Kind::F32));
        assert_eq!(sample_kind(WAVE_FORMAT_IEEE_FLOAT, 32, None), Some(Kind::F32));
        assert_eq!(sample_kind(WAVE_FORMAT_EXTENSIBLE, 24, Some(SUBTYPE_PCM)), Some(Kind::I24));
        assert_eq!(sample_kind(1, 16, None), Some(Kind::I16));
        assert_eq!(sample_kind(WAVE_FORMAT_EXTENSIBLE, 64, Some(SUBTYPE_FLOAT)), None);
        assert_eq!(sample_kind(0x55, 16, None), None, "MP3 and the like");
        let mut out = Vec::new();
        to_f32(Kind::F32, &[0.5f32.to_le_bytes(), (-0.25f32).to_le_bytes()].concat(), 8, 2, &mut out);
        assert_eq!(out, [0.5, -0.25]);
        to_f32(Kind::I16, &[0x00, 0x40, 0x00, 0x80], 4, 2, &mut out);
        assert_eq!(out, [0.5, -1.0]);
        to_f32(Kind::I24, &[0x00, 0x00, 0x40, 0, 0, 0], 6, 1, &mut out);
        assert_eq!(out, [0.5], "padding past the channels is skipped");
    }
}
