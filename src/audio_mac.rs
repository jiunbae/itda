//! This Mac's sound for viewers of its high-performance sharing (audio.rs): ScreenCaptureKit's
//! audio capture (macOS 13+), which Screen Recording permission already covers, so it asks
//! for nothing new. Itda's own sounds are left out.
//!
//! It runs on a stream of its own rather than the screen's (host_mac.rs): it starts and stops
//! with listening viewers instead of with the picture, and neither can take the other down.
//! ScreenCaptureKit has no audio-only stream, so its picture is 2×2 once a second.

use crate::audio::{Capture, Sink};
use block2::RcBlock;
use dispatch2::DispatchQueue;
use objc2::rc::Retained;
use objc2::runtime::{NSObject, NSObjectProtocol, ProtocolObject};
use objc2::{define_class, msg_send, AllocAnyThread, DefinedClass, Message};
use objc2_core_audio_types::{kAudioFormatFlagIsFloat, AudioBuffer, AudioBufferList};
use objc2_core_foundation::CFRetained;
use objc2_core_media::{kCMSampleBufferFlag_AudioBufferList_Assure16ByteAlignment, CMAudioFormatDescriptionGetStreamBasicDescription, CMBlockBuffer, CMSampleBuffer, CMTime};
use objc2_core_graphics::{CGMainDisplayID, CGPreflightScreenCaptureAccess};
use objc2_foundation::{NSArray, NSError, NSProcessInfo};
use objc2_screen_capture_kit::{SCContentFilter, SCShareableContent, SCStream, SCStreamConfiguration, SCStreamDelegate, SCStreamOutput, SCStreamOutputType, SCWindow};
use std::ptr::NonNull;
use std::sync::Mutex;
use std::time::Duration;

const WAIT: Duration = Duration::from_secs(5);

/// macOS 13 or later (ScreenCaptureKit audio), with screen capture available and allowed.
pub(crate) fn audio_supported() -> bool {
    NSProcessInfo::processInfo().operatingSystemVersion().majorVersion >= 13 && crate::host_mac::capture_support().is_ok() && CGPreflightScreenCaptureAccess()
}

/// Start capturing on a thread of its own; dropping the result stops it.
pub(crate) fn audio_capture(sink: Sink) -> Option<Capture> {
    if !audio_supported() {
        return None;
    }
    let (stop, stopped) = std::sync::mpsc::channel::<()>();
    std::thread::spawn(move || match start(sink) {
        Ok((stream, _output)) => {
            let _ = stopped.recv(); // (the sender dropped: stop)
            unsafe { stream.stopCaptureWithCompletionHandler(None) };
        }
        Err(e) => crate::log!("hp: sound capture didn't start: {e}"),
    });
    Some(Box::new(stop))
}

struct Ivars {
    sink: Mutex<Sink>,
    buf: Mutex<Vec<f32>>,
}

define_class!(
    #[unsafe(super(NSObject))]
    #[name = "ItdaAudioOutput"]
    #[ivars = Ivars]
    struct Output;
    unsafe impl NSObjectProtocol for Output {}
    unsafe impl SCStreamOutput for Output {
        #[unsafe(method(stream:didOutputSampleBuffer:ofType:))]
        fn sample(&self, _stream: &SCStream, sample: &CMSampleBuffer, kind: SCStreamOutputType) {
            if kind == SCStreamOutputType::Audio {
                let mut buf = self.ivars().buf.lock().unwrap();
                if let Some((rate, ch)) = unsafe { samples(sample, &mut buf) } {
                    (self.ivars().sink.lock().unwrap())(&buf, rate, ch);
                }
            }
        }
    }
    unsafe impl SCStreamDelegate for Output {
        #[unsafe(method(stream:didStopWithError:))]
        fn stopped(&self, _stream: &SCStream, error: &NSError) {
            crate::log!("hp: sound capture stopped: {}", error.localizedDescription());
        }
    }
);

impl Output {
    fn new(sink: Sink) -> Retained<Self> {
        let this = Self::alloc().set_ivars(Ivars { sink: Mutex::new(sink), buf: Mutex::new(Vec::new()) });
        unsafe { msg_send![super(this), init] }
    }
}

/// An AudioBufferList with room for 8 buffers (one per channel, non-interleaved).
#[repr(C)]
struct List {
    n: u32,
    buffers: [AudioBuffer; 8],
}

/// The sample buffer's float samples, interleaved into `out`: (rate, channels).
unsafe fn samples(sample: &CMSampleBuffer, out: &mut Vec<f32>) -> Option<(u32, usize)> {
    let desc = sample.format_description()?;
    let asbd = CMAudioFormatDescriptionGetStreamBasicDescription(&desc).as_ref()?;
    if asbd.mFormatFlags & kAudioFormatFlagIsFloat == 0 || asbd.mBitsPerChannel != 32 {
        return None;
    }
    let mut list: List = std::mem::zeroed();
    let mut block: *mut CMBlockBuffer = std::ptr::null_mut();
    // The list's size must be exactly what it needs (a larger one is "array too small").
    let mut need = 0usize;
    let flags = kCMSampleBufferFlag_AudioBufferList_Assure16ByteAlignment;
    if sample.audio_buffer_list_with_retained_block_buffer(&mut need, std::ptr::null_mut(), 0, None, None, flags, std::ptr::null_mut()) != 0 || need > size_of::<List>() {
        return None;
    }
    let status = sample.audio_buffer_list_with_retained_block_buffer(
        std::ptr::null_mut(),
        &mut list as *mut List as *mut AudioBufferList,
        need,
        None,
        None,
        flags,
        &mut block,
    );
    let _block = NonNull::new(block).map(|b| CFRetained::from_raw(b)); // keeps the data alive
    if status != 0 || list.n == 0 || list.n as usize > list.buffers.len() {
        return None;
    }
    let buffers = &list.buffers[..list.n as usize];
    let ch: usize = buffers.iter().map(|b| b.mNumberChannels as usize).sum();
    let frames = buffers.iter().map(|b| b.mDataByteSize as usize / 4 / (b.mNumberChannels as usize).max(1)).min()?;
    if ch == 0 || frames == 0 || buffers.iter().any(|b| b.mData.is_null()) {
        return None;
    }
    out.clear();
    out.resize(frames * ch, 0.0);
    let mut first = 0;
    for b in buffers {
        let n = b.mNumberChannels as usize;
        let data = std::slice::from_raw_parts(b.mData as *const f32, frames * n);
        for f in 0..frames {
            out[f * ch + first..f * ch + first + n].copy_from_slice(&data[f * n..f * n + n]);
        }
        first += n;
    }
    Some((asbd.mSampleRate as u32, ch))
}

fn start(sink: Sink) -> Result<(Retained<SCStream>, Retained<Output>), String> {
    let (tx, rx) = std::sync::mpsc::channel::<Option<Retained<SCShareableContent>>>();
    let handler = RcBlock::new(move |content: *mut SCShareableContent, _: *mut NSError| {
        let _ = tx.send(unsafe { content.as_ref() }.map(|c| c.retain()));
    });
    unsafe { SCShareableContent::getShareableContentWithCompletionHandler(&handler) };
    let content = rx.recv_timeout(WAIT).ok().flatten().ok_or("no shareable content")?;
    let main = CGMainDisplayID();
    let displays = unsafe { content.displays() };
    let display = displays.iter().find(|d| unsafe { d.displayID() } == main).or_else(|| displays.iter().next()).ok_or("no display")?;
    unsafe {
        let filter = SCContentFilter::initWithDisplay_excludingWindows(SCContentFilter::alloc(), &display, &NSArray::<SCWindow>::new());
        let config = SCStreamConfiguration::new();
        config.setWidth(2);
        config.setHeight(2);
        config.setMinimumFrameInterval(CMTime::new(1, 1));
        config.setShowsCursor(false);
        config.setCapturesAudio(true);
        config.setSampleRate(crate::audio::RATE as isize);
        config.setChannelCount(2);
        config.setExcludesCurrentProcessAudio(true);
        let output = Output::new(sink);
        let stream = SCStream::initWithFilter_configuration_delegate(SCStream::alloc(), &filter, &config, Some(ProtocolObject::from_ref(&*output)));
        let queue = DispatchQueue::new("itda.audio", None);
        // The (unused) picture too: without an output for it, ScreenCaptureKit logs each frame.
        for kind in [SCStreamOutputType::Audio, SCStreamOutputType::Screen] {
            stream.addStreamOutput_type_sampleHandlerQueue_error(ProtocolObject::from_ref(&*output), kind, Some(&queue)).map_err(|e| e.localizedDescription().to_string())?;
        }
        let (tx, rx) = std::sync::mpsc::channel::<Option<String>>();
        let done = RcBlock::new(move |error: *mut NSError| {
            let _ = tx.send(error.as_ref().map(|e| e.localizedDescription().to_string()));
        });
        stream.startCaptureWithCompletionHandler(Some(&done));
        match rx.recv_timeout(WAIT) {
            Ok(None) => Ok((stream, output)),
            Ok(Some(e)) => Err(e),
            Err(_) => {
                stream.stopCaptureWithCompletionHandler(None);
                Err("ScreenCaptureKit didn't answer".into())
            }
        }
    }
}

#[cfg(test)]
mod tests {
    /// The real capture: a short system sound comes through as samples. Opt-in (it plays a
    /// sound, quietly), and skipped without asking where Screen Recording isn't already allowed.
    /// cargo test --release mac_sound_capture -- --ignored --nocapture
    #[test]
    #[ignore]
    fn mac_sound_capture() {
        if !super::audio_supported() {
            return println!("skipped: needs macOS 13+ and Screen Recording already allowed");
        }
        let (tx, rx) = std::sync::mpsc::channel();
        let mut ingest = crate::audio::Ingest::default();
        let capture = super::audio_capture(Box::new(move |s, rate, ch| {
            if let Some(c) = ingest.push(s, rate, ch) {
                let _ = tx.send((rate, ch, c.len() / 2, c.iter().fold(0f32, |m, x| m.max(x.abs()))));
            }
        }))
        .unwrap();
        std::thread::sleep(std::time::Duration::from_secs(2)); // (the stream starting)
        let _ = std::process::Command::new("/usr/bin/afplay").args(["-v", "0.15", "/System/Library/Sounds/Tink.aiff"]).status();
        let got: Vec<_> = rx.try_iter().collect();
        drop(capture);
        let frames: usize = got.iter().map(|g| g.2).sum();
        println!("{} chunks, {frames} frames at 48 kHz, source {:?}, peak {:.3}", got.len(), got.first().map(|g| (g.0, g.1)), got.iter().map(|g| g.3).fold(0.0, f32::max));
        assert!(frames > 1000, "the sound was captured");
    }
}
