//! Screen capture through DXGI Desktop Duplication: the compositor hands over each
//! display's new frame and what changed in it, so a capture costs about a copy of the
//! changed rows. GDI's BitBlt of the whole desktop takes ~60 ms for two 4K displays, which
//! caps sharing at ~9 frames a second.
//!
//! `Dup::update` keeps a caller's virtual-desktop buffer current; when duplication isn't
//! possible (secure desktop, display mode change, a remote session, a rotated display, a
//! second duplication in this process) it fails and the caller falls back to GDI. HDR and
//! 10-bit desktops are handed over converted to 8-bit BGRA.

use windows::core::Interface;
use windows::Win32::Foundation::{HMODULE, RECT};
use windows::Win32::Graphics::Direct3D::D3D_DRIVER_TYPE_UNKNOWN;
use windows::Win32::Graphics::Direct3D11::*;
use windows::Win32::Graphics::Dxgi::Common::{DXGI_FORMAT_B8G8R8A8_UNORM, DXGI_MODE_ROTATION_IDENTITY, DXGI_MODE_ROTATION_UNSPECIFIED, DXGI_SAMPLE_DESC};
use windows::Win32::Graphics::Dxgi::*;

struct Out {
    dup: IDXGIOutputDuplication,
    device: ID3D11Device,
    ctx: ID3D11DeviceContext,
    staging: Option<ID3D11Texture2D>,
    /// Where this display sits on the desktop.
    rect: RECT,
}

pub struct Dup {
    outs: Vec<Out>,
    rects: Vec<RECT>,
    moves: Vec<DXGI_OUTDUPL_MOVE_RECT>,
}

impl Dup {
    /// Duplicate every display of the desktop; why not if any of them can't be.
    pub fn new() -> Result<Dup, String> {
        let at = |what: &'static str| move |e: windows::core::Error| format!("{what}: {e}");
        unsafe {
            let factory: IDXGIFactory1 = CreateDXGIFactory1().map_err(at("factory"))?;
            let mut outs = Vec::new();
            let mut a = 0;
            while let Ok(adapter) = factory.EnumAdapters1(a) {
                a += 1;
                let mut device = None;
                let mut ctx = None;
                let mut o = 0;
                while let Ok(output) = adapter.EnumOutputs(o) {
                    o += 1;
                    let desc = output.GetDesc().map_err(at("output"))?;
                    if !desc.AttachedToDesktop.as_bool() {
                        continue;
                    }
                    if desc.Rotation != DXGI_MODE_ROTATION_IDENTITY && desc.Rotation != DXGI_MODE_ROTATION_UNSPECIFIED {
                        return Err("rotated display".into());
                    }
                    if device.is_none() {
                        D3D11CreateDevice(&adapter, D3D_DRIVER_TYPE_UNKNOWN, HMODULE::default(), D3D11_CREATE_DEVICE_BGRA_SUPPORT, None, D3D11_SDK_VERSION, Some(&mut device), None, Some(&mut ctx)).map_err(at("device"))?;
                    }
                    let (Some(dev), Some(c)) = (device.clone(), ctx.clone()) else { return Err("device".into()) };
                    let name = String::from_utf16_lossy(&desc.DeviceName).trim_end_matches('\0').to_string();
                    // Asking for 8-bit BGRA only has Windows convert an HDR or 10-bit desktop.
                    let dup = match output.cast::<IDXGIOutput5>() {
                        Ok(o5) => o5.DuplicateOutput1(&dev, 0, &[DXGI_FORMAT_B8G8R8A8_UNORM]),
                        Err(_) => output.cast::<IDXGIOutput1>().map_err(at("output1"))?.DuplicateOutput(&dev),
                    }
                    .map_err(|e| format!("duplicate {name}: {e}"))?;
                    outs.push(Out { dup, device: dev, ctx: c, staging: None, rect: desc.DesktopCoordinates });
                }
            }
            if outs.is_empty() {
                return Err("no displays".into());
            }
            Ok(Dup { outs, rects: Vec::new(), moves: Vec::new() })
        }
    }

    /// Copy what changed on each display since the last call into `dst`, a w×h top-down
    /// 0x00RRGGBB buffer whose (0, 0) is desktop (ox, oy). The first call after `new`
    /// delivers each display whole. Err: duplication is lost; make a new one.
    pub fn update(&mut self, dst: &mut [u32], w: usize, h: usize, ox: i32, oy: i32) -> Result<(), ()> {
        for i in 0..self.outs.len() {
            unsafe { self.update_out(i, dst, w, h, ox, oy)? };
        }
        Ok(())
    }

    unsafe fn update_out(&mut self, i: usize, dst: &mut [u32], w: usize, h: usize, ox: i32, oy: i32) -> Result<(), ()> {
        let mut info = DXGI_OUTDUPL_FRAME_INFO::default();
        let mut res = None;
        let out = &mut self.outs[i];
        match out.dup.AcquireNextFrame(0, &mut info, &mut res) {
            Ok(()) => {}
            Err(e) if e.code() == DXGI_ERROR_WAIT_TIMEOUT => return Ok(()),
            Err(_) => return Err(()),
        }
        let r = (|| -> Result<(), ()> {
            if info.LastPresentTime == 0 {
                return Ok(()); // only the pointer moved: it's drawn by the caller
            }
            let tex: ID3D11Texture2D = res.as_ref().ok_or(())?.cast().map_err(|_| ())?;
            let mut desc = D3D11_TEXTURE2D_DESC::default();
            tex.GetDesc(&mut desc);
            if desc.Format != DXGI_FORMAT_B8G8R8A8_UNORM {
                return Err(()); // (an 8-bit copy was asked for; the caller falls back to GDI)
            }
            let (tw, th) = (desc.Width as i32, desc.Height as i32);
            // What changed: moved areas' destinations and dirty rectangles, in display pixels.
            self.rects.clear();
            let mut whole = true;
            let mut need = 0u32;
            let n = info.TotalMetadataBufferSize as usize;
            if n > 0 {
                self.moves.resize(n / size_of::<DXGI_OUTDUPL_MOVE_RECT>() + 1, DXGI_OUTDUPL_MOVE_RECT::default());
                self.rects.resize(n / size_of::<RECT>() + 1, RECT::default());
                let mb = (self.moves.len() * size_of::<DXGI_OUTDUPL_MOVE_RECT>()) as u32;
                if out.dup.GetFrameMoveRects(mb, self.moves.as_mut_ptr(), &mut need).is_ok() {
                    let moved: Vec<RECT> = self.moves[..need as usize / size_of::<DXGI_OUTDUPL_MOVE_RECT>()].iter().map(|m| m.DestinationRect).collect();
                    let rb = (self.rects.len() * size_of::<RECT>()) as u32;
                    if out.dup.GetFrameDirtyRects(rb, self.rects.as_mut_ptr(), &mut need).is_ok() {
                        self.rects.truncate(need as usize / size_of::<RECT>());
                        self.rects.extend(moved);
                        whole = false;
                    }
                }
            }
            if whole {
                self.rects.clear();
                self.rects.push(RECT { left: 0, top: 0, right: tw, bottom: th });
            }
            if self.rects.is_empty() {
                return Ok(());
            }
            if out.staging.as_ref().is_none_or(|s| {
                let mut d = D3D11_TEXTURE2D_DESC::default();
                s.GetDesc(&mut d);
                (d.Width, d.Height, d.Format) != (desc.Width, desc.Height, desc.Format)
            }) {
                let sd = D3D11_TEXTURE2D_DESC {
                    Width: desc.Width,
                    Height: desc.Height,
                    MipLevels: 1,
                    ArraySize: 1,
                    Format: desc.Format,
                    SampleDesc: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
                    Usage: D3D11_USAGE_STAGING,
                    BindFlags: 0,
                    CPUAccessFlags: D3D11_CPU_ACCESS_READ.0 as u32,
                    MiscFlags: 0,
                };
                let mut s = None;
                out.device.CreateTexture2D(&sd, None, Some(&mut s)).map_err(|_| ())?;
                out.staging = s;
            }
            let staging = out.staging.as_ref().ok_or(())?;
            out.ctx.CopyResource(staging, &tex);
            let mut map = D3D11_MAPPED_SUBRESOURCE::default();
            out.ctx.Map(staging, 0, D3D11_MAP_READ, 0, Some(&mut map)).map_err(|_| ())?;
            let pitch = map.RowPitch as usize / 4;
            let src = std::slice::from_raw_parts(map.pData as *const u32, pitch * th as usize);
            // Display pixel (x, y) is buffer pixel (x + dx, y + dy).
            let (dx, dy) = (out.rect.left - ox, out.rect.top - oy);
            for r in &self.rects {
                let x0 = r.left.max(0).max(-dx);
                let x1 = r.right.min(tw).min(w as i32 - dx);
                let y0 = r.top.max(0).max(-dy);
                let y1 = r.bottom.min(th).min(h as i32 - dy);
                if x0 >= x1 || y0 >= y1 {
                    continue;
                }
                for y in y0..y1 {
                    let s = &src[y as usize * pitch + x0 as usize..y as usize * pitch + x1 as usize];
                    let o = (y + dy) as usize * w + (x0 + dx) as usize;
                    for (d, p) in dst[o..o + s.len()].iter_mut().zip(s) {
                        *d = p & 0x00ff_ffff;
                    }
                }
            }
            out.ctx.Unmap(staging, 0);
            Ok(())
        })();
        drop(res);
        let released = out.dup.ReleaseFrame().is_ok();
        r.and(if released { Ok(()) } else { Err(()) })
    }
}
