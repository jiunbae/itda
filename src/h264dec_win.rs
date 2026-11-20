//! H.264 ("Open H.264", RFB encoding 50) for the Windows viewer through Media Foundation's
//! H.264 decoder MFT: Annex B in, one access unit per sample, NV12 out, converted (and scaled,
//! when the server sent a smaller picture) into the framebuffer (yuv.rs). Itda's own sharing
//! server sends moving screens this way at a fraction of ZRLE's size.
//!
//! In software by default. With the "decode video on the GPU" setting, the same MFT is given a
//! D3D11 device (DXVA): it decodes into textures, and each picture is copied to a staging
//! texture and read from there. Anything going wrong while setting that up falls back to the
//! software decoder at once; failures while decoding are returned as errors, and net.rs falls
//! back from the GPU to software once, and from software to ZRLE, instead of ending the session.
use crate::yuv::{self, Nv12, Yuv};
use std::sync::atomic::{AtomicBool, Ordering::Relaxed};
use std::sync::OnceLock;
use std::time::{Duration, Instant};
use windows::core::Interface;
use windows::Win32::Foundation::HMODULE;
use windows::Win32::Graphics::Direct3D::{D3D_DRIVER_TYPE_HARDWARE, D3D_FEATURE_LEVEL, D3D_FEATURE_LEVEL_10_0, D3D_FEATURE_LEVEL_10_1, D3D_FEATURE_LEVEL_11_0, D3D_FEATURE_LEVEL_11_1};
use windows::Win32::Graphics::Direct3D11::*;
use windows::Win32::Graphics::Dxgi::Common::{DXGI_FORMAT_NV12, DXGI_SAMPLE_DESC};
use windows::Win32::Graphics::Dxgi::IDXGIDevice;
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
            let ok = std::thread::spawn(|| match Decoder::new(1920, 1080, false) {
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

/// The "decode video on the GPU" setting (session::Launch), and whether the GPU path failed in
/// this process (then every later decoder is a software one).
static GPU: AtomicBool = AtomicBool::new(false);
static GPU_OFF: AtomicBool = AtomicBool::new(false);

/// Set from the session's settings. ITDA_GPU_DECODE=1 or 0 overrides it (for testing).
pub fn want_gpu(on: bool) {
    let on = match std::env::var("ITDA_GPU_DECODE").as_deref() {
        Ok("1") => true,
        Ok("0") => false,
        _ => on,
    };
    if on {
        crate::log!("h264: GPU decoding asked for");
    }
    GPU.store(on, Relaxed);
}

/// Whether a new decoder should try the GPU.
pub fn gpu_wanted() -> bool {
    GPU.load(Relaxed) && !GPU_OFF.load(Relaxed)
}

/// The GPU path failed: software decoders from now on, in this process.
pub fn gpu_failed() {
    GPU_OFF.store(true, Relaxed);
}

/// A hardware D3D11 device with video support, and the device manager the decoder MFT takes.
/// Pictures come out in the decoder's textures and are read back through `staging`.
struct Gpu {
    ctx: ID3D11DeviceContext,
    dev: ID3D11Device,
    mgr: IMFDXGIDeviceManager,
    /// CPU-readable copy of one picture (NV12, the decoder textures' size), made when the
    /// first picture comes and again only if their size changes.
    staging: Option<(ID3D11Texture2D, u32, u32)>,
}

impl Gpu {
    fn new() -> Result<Gpu, String> {
        let at = |what: &'static str| move |e: windows::core::Error| format!("{what}: {e}");
        unsafe {
            let (mut dev, mut ctx) = (None, None);
            let levels: [D3D_FEATURE_LEVEL; 4] = [D3D_FEATURE_LEVEL_11_1, D3D_FEATURE_LEVEL_11_0, D3D_FEATURE_LEVEL_10_1, D3D_FEATURE_LEVEL_10_0];
            D3D11CreateDevice(None, D3D_DRIVER_TYPE_HARDWARE, HMODULE::default(), D3D11_CREATE_DEVICE_VIDEO_SUPPORT, Some(&levels), D3D11_SDK_VERSION, Some(&mut dev), None, Some(&mut ctx)).map_err(at("no hardware D3D11 device with video support"))?;
            let (Some(dev), Some(ctx)) = (dev, ctx) else { return Err("no D3D11 device".into()) };
            // Which graphics card; Windows' own software adapter (no driver, e.g. a VM) decodes
            // no faster than the software decoder.
            if let Ok(desc) = dev.cast::<IDXGIDevice>().and_then(|d| d.GetAdapter()).and_then(|a| a.GetDesc()) {
                let name = String::from_utf16_lossy(&desc.Description).trim_end_matches('\0').to_string();
                crate::log!("h264: GPU {name} (vendor {:04x} device {:04x}, {} MB)", desc.VendorId, desc.DeviceId, desc.DedicatedVideoMemory >> 20);
                if desc.VendorId == 0x1414 {
                    return Err(format!("{name} is not a graphics card"));
                }
            }
            // The decoder uses the device from its own threads too.
            match ctx.cast::<ID3D11Multithread>() {
                Ok(m) => {
                    let _ = m.SetMultithreadProtected(true);
                }
                Err(e) => return Err(format!("no multithread protection: {e}")),
            }
            let (mut token, mut mgr) = (0u32, None);
            MFCreateDXGIDeviceManager(&mut token, &mut mgr).map_err(at("device manager"))?;
            let mgr = mgr.ok_or("no device manager")?;
            mgr.ResetDevice(&dev, token).map_err(at("device manager reset"))?;
            Ok(Gpu { ctx, dev, mgr, staging: None })
        }
    }

    /// Copy the picture in a decoder output buffer (a texture, or one slice of a texture array)
    /// to the staging texture and show it to `f` as NV12: its bytes, row pitch and the luma
    /// plane's rows (the chroma plane follows).
    fn read(&mut self, dx: &IMFDXGIBuffer, min_w: usize, f: impl FnOnce(&[u8], usize, usize)) -> windows::core::Result<()> {
        unsafe {
            let mut raw = std::ptr::null_mut();
            dx.GetResource(&ID3D11Texture2D::IID, &mut raw)?;
            let tex = ID3D11Texture2D::from_raw(raw);
            let sub = dx.GetSubresourceIndex()?;
            let mut desc = D3D11_TEXTURE2D_DESC::default();
            tex.GetDesc(&mut desc);
            if desc.Format != DXGI_FORMAT_NV12 {
                return Err(windows::core::Error::new(MF_E_INVALIDMEDIATYPE, format!("decoder texture format {} is not NV12", desc.Format.0)));
            }
            if self.staging.as_ref().is_none_or(|&(_, w, h)| (w, h) != (desc.Width, desc.Height)) {
                self.staging = None;
                let sd = D3D11_TEXTURE2D_DESC {
                    Width: desc.Width,
                    Height: desc.Height,
                    MipLevels: 1,
                    ArraySize: 1,
                    Format: DXGI_FORMAT_NV12,
                    SampleDesc: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
                    Usage: D3D11_USAGE_STAGING,
                    BindFlags: 0,
                    CPUAccessFlags: D3D11_CPU_ACCESS_READ.0 as u32,
                    MiscFlags: 0,
                };
                let mut t = None;
                self.dev.CreateTexture2D(&sd, None, Some(&mut t))?;
                let t = t.ok_or_else(|| windows::core::Error::new(MF_E_UNEXPECTED, "no staging texture"))?;
                crate::log!("h264: GPU pictures {}x{} (texture array of {}), read back through a staging texture", desc.Width, desc.Height, desc.ArraySize);
                self.staging = Some((t, desc.Width, desc.Height));
            }
            let Some((staging, _, rows)) = &self.staging else { unreachable!() };
            self.ctx.CopySubresourceRegion(staging, 0, 0, 0, 0, &tex, sub, None);
            let mut map = D3D11_MAPPED_SUBRESOURCE::default();
            self.ctx.Map(staging, 0, D3D11_MAP_READ, 0, Some(&mut map))?;
            let (pitch, rows) = (map.RowPitch as usize, *rows as usize);
            if map.pData.is_null() || pitch < min_w {
                self.ctx.Unmap(staging, 0);
                return Err(windows::core::Error::new(MF_E_UNEXPECTED, format!("staging pitch {pitch} for {min_w} pixels")));
            }
            // NV12 maps as one block: the luma rows, then the chroma rows (half as many).
            f(std::slice::from_raw_parts(map.pData as *const u8, pitch * rows + pitch * rows.div_ceil(2)), pitch, rows);
            self.ctx.Unmap(staging, 0);
        }
        Ok(())
    }
}

pub struct Decoder {
    t: IMFTransform,
    /// Decoding on the GPU (dropped after the MFT, which holds its device manager).
    gpu: Option<Gpu>,
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

fn open(w: usize, h: usize, gpu: Option<&Gpu>) -> windows::core::Result<IMFTransform> {
    unsafe {
        let t: IMFTransform = CoCreateInstance(&CLSID_MSH264DecoderMFT, None, CLSCTX_INPROC_SERVER)?;
        // Without it the decoder keeps pictures back for reordering the server never does.
        let attrs = t.GetAttributes();
        match &attrs {
            Ok(a) => {
                if let Err(e) = a.SetUINT32(&MF_LOW_LATENCY, 1) {
                    crate::log!("h264: decoder refused low latency mode: {e}");
                }
            }
            Err(e) => crate::log!("h264: decoder has no attributes for low latency: {e}"),
        }
        // On the GPU: the device manager goes in before the media types, which the decoder
        // then checks against what the hardware can do.
        if let Some(g) = gpu {
            let aware = attrs.as_ref().ok().and_then(|a| a.GetUINT32(&MF_SA_D3D11_AWARE).ok()).unwrap_or(0);
            if aware == 0 {
                return Err(windows::core::Error::new(MF_E_UNSUPPORTED_D3D_TYPE, "decoder is not D3D11-aware"));
            }
            t.ProcessMessage(MFT_MESSAGE_SET_D3D_MANAGER, g.mgr.as_raw() as usize)?;
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
    /// A decoder for a w×h rectangle; on the GPU if `gpu` and it can be set up there, else (and
    /// saying why) in software.
    pub fn new(w: usize, h: usize, gpu: bool) -> Result<Decoder, String> {
        if !crate::h264_win::mf_ready() {
            return Err("Media Foundation unavailable".into());
        }
        if gpu {
            match Gpu::new().and_then(|g| Decoder::make(w, h, Some(g))) {
                Ok(d) => return Ok(d),
                Err(e) => {
                    crate::log!("h264: GPU decoding unavailable, using the software decoder: {e}");
                    gpu_failed();
                }
            }
        }
        Decoder::make(w, h, None)
    }

    pub fn gpu(&self) -> bool {
        self.gpu.is_some()
    }

    fn make(w: usize, h: usize, gpu: Option<Gpu>) -> Result<Decoder, String> {
        let on = if gpu.is_some() { " on the GPU" } else { "" };
        let t = open(w.max(16), h.max(16), gpu.as_ref()).map_err(|e| format!("H.264 decoder{on}: {e}"))?;
        let mut d = Decoder {
            t,
            gpu,
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
        d.configure().map_err(|e| format!("H.264 decoder{on} output: {e}"))?;
        unsafe {
            d.t.ProcessMessage(MFT_MESSAGE_NOTIFY_BEGIN_STREAMING, 0).map_err(|e| format!("H.264 decoder{on} start: {e}"))?;
            d.t.ProcessMessage(MFT_MESSAGE_NOTIFY_START_OF_STREAM, 0).map_err(|e| format!("H.264 decoder{on} start: {e}"))?;
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
                if self.gpu.is_some() { ", GPU textures" } else if self.provides { ", decoder's samples" } else { "" },
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
        if self.gpu.is_some() {
            self.out = None; // (its texture back to the decoder's pool)
        }
        self.frames += 1;
        self.busy += start.elapsed();
        if self.frames == 1 || self.frames % 600 == 0 {
            let on = if self.gpu.is_some() { " (GPU)" } else { "" };
            crate::log!("h264: {} pictures{on}, {:.1} ms each on average", self.frames, self.busy.as_secs_f64() * 1000.0 / self.frames as f64);
        }
        Ok(())
    }

    /// Lock the last picture's buffer and show it to `draw`.
    fn picture(&mut self, draw: impl FnOnce(&Nv12, &Yuv)) -> windows::core::Result<()> {
        let Some(sample) = self.out.clone() else { return Ok(()) };
        let (cw, ch) = self.coded;
        let (shown, yuv) = (self.shown, self.yuv);
        unsafe {
            let buf = sample.GetBufferByIndex(0)?;
            if let (Some(g), Ok(dx)) = (&mut self.gpu, buf.cast::<IMFDXGIBuffer>()) {
                return g.read(&dx, shown.0, |all, stride, rows| show(all, stride, rows, shown, &yuv, draw));
            }
            // A 2D buffer knows its pitch; locked as a plain buffer it would be copied to the
            // minimum one first.
            if let Ok(b2) = buf.cast::<IMF2DBuffer>() {
                let (mut p, mut pitch) = (std::ptr::null_mut(), 0i32);
                b2.Lock2D(&mut p, &mut pitch)?;
                let stride = pitch as usize;
                if pitch > 0 && stride >= cw && !p.is_null() {
                    let all = std::slice::from_raw_parts(p, stride * ch * 3 / 2);
                    show(all, stride, ch, shown, &yuv, draw);
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
                show(std::slice::from_raw_parts(p, len as usize), self.stride, ch, shown, &yuv, draw);
            }
            buf.Unlock()?;
        }
        Ok(())
    }

}

/// Show NV12 bytes (`rows` luma rows `stride` apart, then the chroma plane) to `draw`: the
/// `shown` part from the top left.
fn show(all: &[u8], stride: usize, rows: usize, shown: (usize, usize), yuv: &Yuv, draw: impl FnOnce(&Nv12, &Yuv)) {
    let (y, uv) = all.split_at((stride * rows).min(all.len()));
    draw(&Nv12 { y, uv, stride, w: shown.0, h: shown.1.min(rows) }, yuv);
}

impl Drop for Decoder {
    fn drop(&mut self) {
        unsafe {
            let _ = self.t.ProcessMessage(MFT_MESSAGE_NOTIFY_END_OF_STREAM, 0);
            let _ = self.t.ProcessMessage(MFT_MESSAGE_NOTIFY_END_STREAMING, 0);
            if self.gpu.is_some() {
                let _ = self.t.ProcessMessage(MFT_MESSAGE_SET_D3D_MANAGER, 0);
            }
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
        // 810 lines: a 1080p screen's 3/4 step, not whole macroblocks (cropped in the SPS).
        for (w, h) in [(640, 360), (1440, 810)] {
            decode_round_trip(w, h);
        }
    }

    fn decode_round_trip(w: usize, h: usize) {
        use crate::server::VideoEncode;
        let mut e = match crate::h264_win::H264::new(w, h, 8_000_000) {
            Ok(e) => e,
            Err(why) => return eprintln!("skipped: no Media Foundation H.264 encoder here ({why})"),
        };
        let mut d = match Decoder::new(w, h, false) {
            Ok(d) => d,
            Err(why) => return eprintln!("skipped: no Media Foundation H.264 decoder here ({why})"),
        };
        // Broad bands: greys, then red, green and blue, which come back as they went only when
        // the decoder converts with the matrix the encoder used.
        const BANDS: [u32; 7] = [0x000000, 0x404040, 0xa0a0a0, 0xffffff, 0xd02020, 0x20b040, 0x3050e0];
        let px: Vec<u32> = (0..w * h).map(|i| BANDS[(i % w) * BANDS.len() / w]).collect();
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
        assert!(pictures >= 2, "{w}x{h}: low latency: a picture per input ({pictures})");
        for (i, want) in BANDS.iter().enumerate() {
            let x = (2 * i + 1) * w / (2 * BANDS.len());
            for y in [h / 2, h - 4] {
                let got = out[y * w + x];
                let close = (0..3).all(|c| (((got >> (c * 8)) & 255) as i32 - ((want >> (c * 8)) & 255) as i32).abs() <= 12);
                assert!(close, "{w}x{h} at {x},{y}: {got:06x} vs {want:06x}");
            }
        }
    }
}
