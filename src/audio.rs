//! Remote audio for the share server (server.rs): this machine's sound output — a loopback of
//! what plays, never the microphone — for viewers that ask for it. The platform captures it
//! (`audio_capture`: WASAPI loopback on Windows, ScreenCaptureKit on macOS 13+); this module
//! has the wire protocol, the format conversion and the fan-out to viewers.
//!
//! Wire: the QEMU Audio extension of RFB. A viewer lists pseudo-encoding -259; the server
//! confirms with an empty rectangle of it in a FramebufferUpdate; the viewer then sets a sample
//! format and enables audio (client message 255, submessage 1), and gets stream begin, data and
//! end (server message 255, submessage 1). Samples are little-endian, as QEMU sends its host's.
//! Raw PCM: 48 kHz stereo S16 is about 1.5 Mbit/s, fine on a LAN or tailnet, with no codec.
//!
//! One capture serves every listening viewer, started with the first and stopped with the
//! last. It is resampled once to 48 kHz stereo float; each viewer converts that to its own
//! format. Silence sends nothing (a stream end after END_AFTER).
//!
//! Audio never holds up the picture or input: each viewer has a short queue that drops its
//! oldest audio (QUEUE_MAX), and its pump sends audio only while the outbox has room, so a
//! slow link loses sound rather than delays the screen. The reader thread only parses.

use std::collections::VecDeque;
use std::io::{self, Read};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

pub const ENC_QEMU_AUDIO: i32 = -259;
/// QEMU client and server message type.
pub const MSG_QEMU: u8 = 255;
const SUB_KEY: u8 = 0;
const SUB_AUDIO: u8 = 1;
/// The rate and channel count everything is converted to first.
pub const RATE: u32 = 48_000;
/// Sample formats (QEMU's numbering): U8, S8, U16, S16, U32, S32.
pub const S16: u8 = 3;
/// Audio a viewer may have waiting: 200 ms; older audio is dropped as new arrives.
const QUEUE_MAX: usize = RATE as usize / 5;
/// Silence this long ends the stream (the viewer's player can stop and rebuffer).
const END_AFTER: Duration = Duration::from_millis(300);

/// What the platform hands over: interleaved float samples, their rate and channel count.
pub(crate) type Sink = Box<dyn FnMut(&[f32], u32, usize) + Send>;
/// A running platform capture; dropping it stops it (it may finish on its own thread).
pub(crate) type Capture = Box<dyn Send>;

// ---------------------------------------------------------------- protocol

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Format {
    pub sample: u8,
    pub channels: u8,
    pub freq: u32,
}

impl Format {
    pub const DEFAULT: Format = Format { sample: S16, channels: 2, freq: RATE };

    fn valid(&self) -> bool {
        self.sample <= 5 && (1..=2).contains(&self.channels) && (8_000..=192_000).contains(&self.freq)
    }

    fn bytes(&self) -> usize {
        [1, 1, 2, 2, 4, 4][self.sample as usize]
    }
}

/// A viewer's audio request.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Op {
    Enable,
    Disable,
    Format(Format),
}

/// Read the rest of a QEMU client message (its type byte already read): an audio request, or
/// None for one to ignore (an extended key event: never confirmed, but harmless).
pub fn read(r: &mut impl Read) -> io::Result<Option<Op>> {
    let mut h = [0u8; 3];
    r.read_exact(&mut h)?;
    let op = u16::from_be_bytes([h[1], h[2]]);
    match h[0] {
        SUB_KEY => {
            r.read_exact(&mut [0u8; 8])?; // keysym, keycode (the down flag was `op`)
            Ok(None)
        }
        SUB_AUDIO => match op {
            0 => Ok(Some(Op::Enable)),
            1 => Ok(Some(Op::Disable)),
            2 => {
                let mut b = [0u8; 6];
                r.read_exact(&mut b)?;
                Ok(Some(Op::Format(Format { sample: b[0], channels: b[1], freq: u32::from_be_bytes([b[2], b[3], b[4], b[5]]) })))
            }
            n => Err(io::Error::other(format!("unknown QEMU audio operation {n}"))),
        },
        s => Err(io::Error::other(format!("unknown QEMU client message {s}"))),
    }
}

/// Server message: stream begin (1), end (0).
fn op_msg(out: &mut Vec<u8>, op: u16) {
    out.extend_from_slice(&[MSG_QEMU, SUB_AUDIO]);
    out.extend_from_slice(&op.to_be_bytes());
}

/// The empty pseudo-rectangle that tells the viewer this server sends audio.
pub fn confirm_rect(out: &mut Vec<u8>) {
    out.extend_from_slice(&[0; 8]);
    out.extend_from_slice(&ENC_QEMU_AUDIO.to_be_bytes());
}

// ---------------------------------------------------------------- conversion

/// Fold a frame of `ch` channels (Windows' WAVEFORMATEXTENSIBLE order) into stereo: the
/// centre and surrounds go into both sides, so a film's dialogue isn't lost; LFE is left out.
fn stereo(f: &[f32]) -> (f32, f32) {
    const K: f32 = std::f32::consts::FRAC_1_SQRT_2;
    match f.len() {
        0 => (0.0, 0.0),
        1 => (f[0], f[0]),
        2 => (f[0], f[1]),
        3 => (f[0] + K * f[2], f[1] + K * f[2]),
        4 => (f[0] + K * f[2], f[1] + K * f[3]), // quad: FL FR BL BR
        n => {
            // FL FR FC LFE BL BR (5.1), then SL SR (7.1).
            let (mut l, mut r) = (f[0] + K * f[2], f[1] + K * f[2]);
            l += K * f[4];
            if n > 5 {
                r += K * f[5];
            }
            if n >= 8 {
                l += K * f[6];
                r += K * f[7];
            }
            (l, r)
        }
    }
}

/// Linear interpolation from one rate to another, of interleaved stereo, continuing across
/// chunks (one frame of delay). Plenty for monitoring a remote machine's sound.
pub struct Resampler {
    step: f64,
    /// Position of the next output frame, counted from `last` (0) into the next chunk (1..).
    t: f64,
    last: [f32; 2],
}

impl Resampler {
    pub fn new(from: u32, to: u32) -> Resampler {
        Resampler { step: from as f64 / to as f64, t: 1.0, last: [0.0; 2] }
    }

    pub fn run(&mut self, input: &[f32], out: &mut Vec<f32>) {
        let n = input.len() / 2;
        if n == 0 {
            return;
        }
        let last = self.last;
        let at = |i: usize, c: usize| if i == 0 { last[c] } else { input[(i - 1) * 2 + c] };
        let mut t = self.t;
        while t < n as f64 {
            let i = t as usize;
            let k = (t - i as f64) as f32;
            for c in 0..2 {
                out.push(at(i, c) + (at(i + 1, c) - at(i, c)) * k);
            }
            t += self.step;
        }
        self.t = t - n as f64;
        self.last = [input[(n - 1) * 2], input[(n - 1) * 2 + 1]];
    }
}

/// Below half a 16-bit step: nothing anyone would hear, and nothing worth sending.
fn silent(s: &[f32]) -> bool {
    s.iter().all(|x| x.abs() < 1.0 / 65536.0)
}

/// The capture side: the platform's samples, whatever they are, into 48 kHz stereo.
pub struct Ingest {
    from: (u32, usize),
    rs: Resampler,
    tmp: Vec<f32>,
}

impl Default for Ingest {
    fn default() -> Ingest {
        Ingest { from: (RATE, 2), rs: Resampler::new(RATE, RATE), tmp: Vec::new() }
    }
}

impl Ingest {
    /// 48 kHz stereo for these samples, or None when they are silent.
    pub fn push(&mut self, input: &[f32], rate: u32, channels: usize) -> Option<Vec<f32>> {
        if channels == 0 || rate == 0 || input.len() < channels {
            return None;
        }
        if silent(input) {
            self.rs = Resampler::new(rate, RATE); // (a fresh start, not a glide from old sound)
            return None;
        }
        if self.from != (rate, channels) {
            self.from = (rate, channels);
            self.rs = Resampler::new(rate, RATE);
        }
        self.tmp.clear();
        for f in input.chunks_exact(channels) {
            let (l, r) = stereo(f);
            self.tmp.extend_from_slice(&[l, r]);
        }
        let mut out = Vec::with_capacity(self.tmp.len() * RATE as usize / rate as usize + 4);
        self.rs.run(&self.tmp, &mut out);
        Some(out)
    }
}

/// The viewer side: 48 kHz stereo into the format it asked for.
pub struct Converter {
    fmt: Format,
    rs: Option<Resampler>,
    tmp: Vec<f32>,
}

impl Converter {
    pub fn new(fmt: Format) -> Converter {
        Converter { fmt, rs: (fmt.freq != RATE).then(|| Resampler::new(RATE, fmt.freq)), tmp: Vec::new() }
    }

    /// Append `stereo` (48 kHz) in this viewer's format: whole frames only.
    pub fn convert(&mut self, stereo: &[f32], out: &mut Vec<u8>) {
        let src = match &mut self.rs {
            Some(rs) => {
                self.tmp.clear();
                rs.run(stereo, &mut self.tmp);
                &self.tmp[..]
            }
            None => stereo,
        };
        let f = self.fmt;
        out.reserve(src.len() / 2 * f.channels as usize * f.bytes());
        for fr in src.chunks_exact(2) {
            if f.channels == 1 {
                sample(out, f.sample, (fr[0] + fr[1]) * 0.5);
            } else {
                sample(out, f.sample, fr[0]);
                sample(out, f.sample, fr[1]);
            }
        }
    }
}

fn sample(out: &mut Vec<u8>, kind: u8, x: f32) {
    let x = x.clamp(-1.0, 1.0) as f64;
    let s8 = (x * 127.0).round() as i8;
    let s16 = (x * 32767.0).round() as i16;
    let s32 = (x * 2147483647.0).round() as i32;
    match kind {
        0 => out.push((s8 as u8) ^ 0x80),
        1 => out.push(s8 as u8),
        2 => out.extend_from_slice(&((s16 as u16) ^ 0x8000).to_le_bytes()),
        3 => out.extend_from_slice(&s16.to_le_bytes()),
        4 => out.extend_from_slice(&((s32 as u32) ^ 0x8000_0000).to_le_bytes()),
        _ => out.extend_from_slice(&s32.to_le_bytes()),
    }
}

// ---------------------------------------------------------------- fan-out

type Wake = Box<dyn Fn() + Send + Sync>;

/// One viewer's waiting audio (48 kHz stereo chunks), at most QUEUE_MAX frames.
pub struct Queue {
    chunks: Mutex<(VecDeque<Arc<[f32]>>, usize)>,
    wake: Wake,
}

impl Queue {
    fn new(wake: Wake) -> Queue {
        Queue { chunks: Mutex::new((VecDeque::new(), 0)), wake }
    }

    fn push(&self, c: Arc<[f32]>) {
        {
            let mut q = self.chunks.lock().unwrap();
            q.1 += c.len() / 2;
            q.0.push_back(c);
            while q.1 > QUEUE_MAX && q.0.len() > 1 {
                let old = q.0.pop_front().unwrap();
                q.1 -= old.len() / 2;
            }
        }
        (self.wake)();
    }

    fn take(&self) -> VecDeque<Arc<[f32]>> {
        let mut q = self.chunks.lock().unwrap();
        q.1 = 0;
        std::mem::take(&mut q.0)
    }

    #[cfg(test)]
    fn frames(&self) -> usize {
        self.chunks.lock().unwrap().1
    }
}

/// The one capture and the viewers listening to it.
pub struct Hub {
    start: fn(Sink) -> Option<Capture>,
    inner: Mutex<HubInner>,
}

struct HubInner {
    subs: Vec<(u64, Arc<Queue>)>,
    capture: Option<Capture>,
    /// Which capture may publish: an old one still stopping on its thread may not.
    generation: u64,
    next: u64,
}

impl Hub {
    pub const fn new(start: fn(Sink) -> Option<Capture>) -> Hub {
        Hub { start, inner: Mutex::new(HubInner { subs: Vec::new(), capture: None, generation: 0, next: 0 }) }
    }

    /// Listen; the capture starts with the first listener (or again, if it couldn't before).
    pub fn subscribe(&'static self, wake: Wake) -> Subscription {
        let queue = Arc::new(Queue::new(wake));
        let mut g = self.inner.lock().unwrap();
        let id = g.next;
        g.next += 1;
        g.subs.push((id, queue.clone()));
        if g.capture.is_none() {
            g.generation += 1;
            let generation = g.generation;
            let mut ingest = Ingest::default();
            g.capture = (self.start)(Box::new(move |s, rate, ch| {
                if let Some(c) = ingest.push(s, rate, ch) {
                    self.publish(generation, c.into());
                }
            }));
            if g.capture.is_none() {
                crate::log!("share: this {} can't capture its sound", if cfg!(windows) { "PC" } else { "Mac" });
            }
        }
        Subscription { hub: self, id, queue }
    }

    fn publish(&self, generation: u64, c: Arc<[f32]>) {
        let g = self.inner.lock().unwrap();
        if g.generation == generation {
            for (_, q) in &g.subs {
                q.push(c.clone());
            }
        }
    }
}

pub struct Subscription {
    hub: &'static Hub,
    id: u64,
    queue: Arc<Queue>,
}

impl Drop for Subscription {
    fn drop(&mut self) {
        let stop = {
            let mut g = self.hub.inner.lock().unwrap();
            g.subs.retain(|(i, _)| *i != self.id);
            if g.subs.is_empty() {
                g.generation += 1;
                g.capture.take()
            } else {
                None
            }
        };
        drop(stop); // (outside the lock: a capture thread may be publishing)
    }
}

#[cfg(any(windows, target_os = "macos"))]
pub static HUB: Hub = Hub::new(crate::server::platform::audio_capture);

// ---------------------------------------------------------------- one viewer

/// One viewer's audio, driven by its pump.
pub struct Stream {
    hub: &'static Hub,
    supported: bool,
    /// The viewer lists -259 / has been sent the confirmation.
    wants: bool,
    confirmed: bool,
    conv: Converter,
    sub: Option<Subscription>,
    wake: Arc<dyn Fn() + Send + Sync>,
    playing: bool,
    last_data: Instant,
}

impl Stream {
    pub fn new(hub: &'static Hub, supported: bool, wake: Arc<dyn Fn() + Send + Sync>) -> Stream {
        Stream { hub, supported, wants: false, confirmed: false, conv: Converter::new(Format::DEFAULT), sub: None, wake, playing: false, last_data: Instant::now() }
    }

    /// The viewer's SetEncodings. One that no longer lists audio stops getting it.
    pub fn encodings(&mut self, encs: &[i32]) {
        self.wants = self.supported && encs.contains(&ENC_QEMU_AUDIO);
        if !self.wants {
            (self.sub, self.playing) = (None, false);
        }
    }

    /// Add the confirmation to the update being built, once: true if it was added.
    pub fn confirm(&mut self, buf: &mut Vec<u8>) -> bool {
        if !self.wants || self.confirmed {
            return false;
        }
        confirm_rect(buf);
        self.confirmed = true;
        true
    }

    #[cfg(test)]
    fn listening(&self) -> bool {
        self.sub.is_some()
    }

    pub fn op(&mut self, op: Op, out: &mut Vec<u8>) {
        match op {
            Op::Format(f) if f.valid() => self.conv = Converter::new(f),
            Op::Format(f) => crate::log!("share: audio format not supported: {f:?}"),
            // (Only after the confirmation, as the extension says.)
            Op::Enable if self.confirmed && self.wants && self.sub.is_none() => {
                let wake = self.wake.clone();
                self.sub = Some(self.hub.subscribe(Box::new(move || wake())));
            }
            Op::Enable => {}
            Op::Disable => {
                self.sub = None;
                if std::mem::take(&mut self.playing) {
                    op_msg(out, 0);
                }
            }
        }
    }

    /// Append what to send now: waiting audio while `room` (else it waits, its oldest
    /// dropped as new comes), and the stream's begin/end around it.
    pub fn pump(&mut self, room: bool, now: Instant, out: &mut Vec<u8>) {
        let Some(sub) = &self.sub else { return };
        if room {
            for c in sub.queue.take() {
                if !std::mem::replace(&mut self.playing, true) {
                    op_msg(out, 1);
                }
                op_msg(out, 2);
                let len = out.len();
                out.extend_from_slice(&[0; 4]);
                self.conv.convert(&c, out);
                let n = (out.len() - len - 4) as u32;
                out[len..len + 4].copy_from_slice(&n.to_be_bytes());
                self.last_data = now;
            }
        }
        if self.playing && now.duration_since(self.last_data) >= END_AFTER {
            self.playing = false;
            op_msg(out, 0);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering::SeqCst};

    #[test]
    fn client_messages() {
        let parse = |b: &[u8]| read(&mut &b[..]);
        assert_eq!(parse(&[1, 0, 0]).unwrap(), Some(Op::Enable));
        assert_eq!(parse(&[1, 0, 1]).unwrap(), Some(Op::Disable));
        let f = parse(&[1, 0, 2, 3, 2, 0, 0, 0xbb, 0x80]).unwrap();
        assert_eq!(f, Some(Op::Format(Format::DEFAULT)));
        // An extended key event is read whole and ignored; what follows stays in place.
        let mut r: &[u8] = &[0, 0, 1, 0, 0, 0, 0x61, 0, 0, 0, 0x1e, 9];
        assert_eq!(read(&mut r).unwrap(), None);
        assert_eq!(r, &[9]);
        assert!(parse(&[1, 0, 7]).is_err());
        assert!(parse(&[2, 0, 0]).is_err());
        assert!(parse(&[1, 0, 2, 3]).is_err(), "cut short");
    }

    #[test]
    fn formats() {
        assert!(Format::DEFAULT.valid());
        assert!(!Format { sample: 6, ..Format::DEFAULT }.valid());
        assert!(!Format { channels: 3, ..Format::DEFAULT }.valid());
        assert!(!Format { freq: 1000, ..Format::DEFAULT }.valid());
        let enc = |kind: u8, x: f32| {
            let mut v = Vec::new();
            sample(&mut v, kind, x);
            v
        };
        assert_eq!(enc(3, 1.0), 32767i16.to_le_bytes());
        assert_eq!(enc(3, -1.0), (-32767i16).to_le_bytes());
        assert_eq!(enc(3, 2.0), 32767i16.to_le_bytes(), "clipped");
        assert_eq!(enc(3, 0.5), 16384i16.to_le_bytes());
        assert_eq!(enc(0, 0.0), [0x80]);
        assert_eq!(enc(1, -1.0), [(-127i8) as u8]);
        assert_eq!(enc(2, 0.0), 0x8000u16.to_le_bytes());
        assert_eq!(enc(5, 1.0), i32::MAX.to_le_bytes());
        assert_eq!(enc(4, 0.0), 0x8000_0000u32.to_le_bytes());
        // Mono is the average of both sides.
        let mut c = Converter::new(Format { channels: 1, ..Format::DEFAULT });
        let mut out = Vec::new();
        c.convert(&[0.5, 0.0, -1.0, -1.0], &mut out);
        assert_eq!(out, [8192i16.to_le_bytes(), (-32767i16).to_le_bytes()].concat());
    }

    #[test]
    fn resampling_keeps_rate_and_shape() {
        // Same rate: the samples come out unchanged (the last one with the next chunk).
        let mut r = Resampler::new(RATE, RATE);
        let mut out = Vec::new();
        let a: Vec<f32> = (0..20).map(|i| i as f32 / 20.0).collect();
        r.run(&a[..10], &mut out);
        r.run(&a[10..], &mut out);
        assert_eq!(out, a[..18]);
        // 44.1 → 48 kHz over many chunks: the right number of frames, and a sine stays a sine.
        let sine = |i: usize, rate: f64| (2.0 * std::f64::consts::PI * 1000.0 * i as f64 / rate).sin() as f32;
        let input: Vec<f32> = (0..44_100).flat_map(|i| [sine(i, 44_100.0); 2]).collect();
        let mut r = Resampler::new(44_100, RATE);
        let mut out = Vec::new();
        for c in input.chunks(441 * 2) {
            r.run(c, &mut out);
        }
        let frames = out.len() / 2;
        assert!((frames as i64 - 48_000).abs() <= 2, "{frames}");
        let worst = (100..frames - 100).map(|i| (out[i * 2] - sine(i, 48_000.0)).abs()).fold(0.0, f32::max);
        assert!(worst < 0.01, "worst error {worst}");
        // 48 → 16 kHz for a viewer that asked for it: a third of the frames.
        let mut c = Converter::new(Format { freq: 16_000, ..Format::DEFAULT });
        let mut bytes = Vec::new();
        c.convert(&vec![0.25; 4800 * 2], &mut bytes);
        assert!((bytes.len() / 4).abs_diff(1600) <= 1);
    }

    #[test]
    fn ingest_folds_channels_and_skips_silence() {
        let mut i = Ingest::default();
        assert_eq!(i.push(&[0.0; 960], RATE, 2), None, "silence sends nothing");
        assert_eq!(i.push(&[1e-6; 960], RATE, 2), None);
        // Mono into both sides; 5.1 with only dialogue (centre) into both sides too.
        let m = i.push(&[0.5; 480], RATE, 1).unwrap();
        assert_eq!(m.len(), 2 * 479, "the last frame waits for the next chunk");
        assert!(m[4..].iter().all(|&x| (x - 0.5).abs() < 1e-6));
        let f: Vec<f32> = (0..480).flat_map(|_| [0.0, 0.0, 0.5, 0.9, 0.0, 0.0]).collect();
        let s = i.push(&f, RATE, 6).unwrap();
        assert!(s[4..].iter().all(|&x| (x - 0.5 * std::f32::consts::FRAC_1_SQRT_2).abs() < 1e-6), "centre in both, no LFE");
        assert!((stereo(&[0.1, 0.2, 0.0, 0.0, 0.3, 0.4, 0.5, 0.6]).0 - (0.1 + std::f32::consts::FRAC_1_SQRT_2 * (0.3 + 0.5))).abs() < 1e-6, "7.1: surrounds in");
        assert_eq!(i.push(&[0.5; 3], RATE, 4), None, "not one whole frame");
    }

    #[test]
    fn queue_drops_its_oldest() {
        let woke = Arc::new(AtomicUsize::new(0));
        let w = woke.clone();
        let q = Queue::new(Box::new(move || {
            w.fetch_add(1, SeqCst);
        }));
        for n in 0..30 {
            q.push(vec![n as f32; 960].into()); // 10 ms each
        }
        assert_eq!(woke.load(SeqCst), 30);
        assert_eq!(q.frames(), QUEUE_MAX);
        let got = q.take();
        assert_eq!(got.len(), 20);
        assert_eq!(got[0][0], 10.0, "the newest 200 ms are kept");
        assert_eq!(q.frames(), 0);
        // One chunk bigger than the limit still goes out (the newest is never dropped).
        q.push(vec![1.0; QUEUE_MAX * 4].into());
        assert_eq!(q.take().len(), 1);
    }

    static SINK: Mutex<Option<Sink>> = Mutex::new(None);
    static STARTS: AtomicUsize = AtomicUsize::new(0);
    static STOPS: AtomicUsize = AtomicUsize::new(0);
    struct Fake;
    impl Drop for Fake {
        fn drop(&mut self) {
            STOPS.fetch_add(1, SeqCst);
        }
    }
    fn fake_start(s: Sink) -> Option<Capture> {
        STARTS.fetch_add(1, SeqCst);
        *SINK.lock().unwrap() = Some(s);
        Some(Box::new(Fake))
    }
    fn play(s: &[f32]) {
        if let Some(sink) = SINK.lock().unwrap().as_mut() {
            sink(s, RATE, 2);
        }
    }

    /// The whole viewer path: no audio before the confirmation, begin/data/end on the wire,
    /// one capture shared by viewers and stopped with the last, nothing while behind.
    #[test]
    fn viewer_stream() {
        static HUB: Hub = Hub::new(fake_start);
        let woke = Arc::new(AtomicUsize::new(0));
        let w = woke.clone();
        let wake: Arc<dyn Fn() + Send + Sync> = Arc::new(move || {
            w.fetch_add(1, SeqCst);
        });
        let mut s = Stream::new(&HUB, true, wake.clone());
        let mut out = Vec::new();
        s.op(Op::Enable, &mut out);
        assert!(!s.listening(), "not before the viewer lists -259 and is confirmed");
        s.encodings(&[16, ENC_QEMU_AUDIO]);
        let mut rect = Vec::new();
        assert!(s.confirm(&mut rect) && !s.confirm(&mut rect));
        assert_eq!(rect, [0, 0, 0, 0, 0, 0, 0, 0, 0xff, 0xff, 0xfe, 0xfd]);
        s.op(Op::Format(Format::DEFAULT), &mut out);
        s.op(Op::Enable, &mut out);
        assert!(s.listening() && out.is_empty());
        assert_eq!(STARTS.load(SeqCst), 1);
        let mut other = Stream::new(&HUB, true, wake.clone());
        other.encodings(&[ENC_QEMU_AUDIO]);
        other.confirm(&mut rect);
        other.op(Op::Enable, &mut out);
        assert_eq!(STARTS.load(SeqCst), 1, "one capture for both");

        play(&[0.0; 960]);
        assert_eq!(woke.load(SeqCst), 0, "silence wakes nobody");
        play(&[0.5; 960]);
        assert_eq!(woke.load(SeqCst), 2);
        let t0 = Instant::now();
        s.pump(false, t0, &mut out);
        assert!(out.is_empty(), "nothing while the outbox is behind");
        s.pump(true, t0, &mut out);
        let data_len = u32::from_be_bytes(out[8..12].try_into().unwrap()) as usize;
        assert_eq!(&out[..8], &[255, 1, 0, 1, 255, 1, 0, 2]);
        assert_eq!(data_len, 479 * 4, "S16 stereo frames (one frame of delay)");
        assert_eq!(out.len(), 12 + data_len);
        assert_eq!(&out[12..16], &[0x00, 0x40, 0x00, 0x40]);
        out.clear();
        s.pump(true, t0 + Duration::from_millis(100), &mut out);
        assert!(out.is_empty());
        s.pump(true, t0 + END_AFTER, &mut out);
        assert_eq!(out, [255, 1, 0, 0], "stream end after silence");
        out.clear();

        // Disable, and a viewer that stops listing audio: the capture stops with the last one.
        s.op(Op::Disable, &mut out);
        assert!(!s.listening() && out.is_empty(), "no end: it had already ended");
        assert_eq!(STOPS.load(SeqCst), 0);
        other.encodings(&[16]);
        assert!(!other.listening());
        assert_eq!(STOPS.load(SeqCst), 1);
        play(&[0.5; 960]); // a late callback from the stopped capture
        s.op(Op::Enable, &mut out);
        assert_eq!(STARTS.load(SeqCst), 2, "started again");
        drop(s);
        assert_eq!(STOPS.load(SeqCst), 2);
        // A platform without audio never confirms.
        let mut none = Stream::new(&HUB, false, wake);
        none.encodings(&[ENC_QEMU_AUDIO]);
        assert!(!none.confirm(&mut rect));
    }
}
