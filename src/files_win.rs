//! Files dragged out of the file browser to Explorer (or any Windows drop target).
//!
//! Files on this PC go as the shell's own data object for the originals; remote files go as
//! virtual files (FileGroupDescriptorW + FileContents) that the receiver reads as streams,
//! pulled over SFTP while it copies. Either way only "copy" is offered: originals are never
//! moved, and the receiver's own name-conflict handling (Explorer asks) applies.
//!
//! The remote data object lives in the multithreaded apartment, so the receiver's reads
//! (Explorer copies on a background thread, see IDataObjectAsyncCapability) run on COM's
//! worker threads instead of blocking this window's thread for the whole download.

use crate::files::{export, vdrag};
use serde_json::{json, Value};
use std::io::Read;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering::Relaxed};
use std::sync::{Arc, Mutex, OnceLock};
use windows::core::{implement, Interface, Ref, BOOL, HRESULT, PCWSTR};
use windows::Win32::Foundation::*;
use windows::Win32::System::Com::Marshal::CoMarshalInterThreadInterfaceInStream;
use windows::Win32::System::Com::StructuredStorage::CoGetInterfaceAndReleaseStream;
use windows::Win32::System::Com::*;
use windows::Win32::System::Memory::{GlobalAlloc, GlobalLock, GlobalUnlock, GlobalSize, GMEM_MOVEABLE, GMEM_ZEROINIT};
use windows::Win32::System::Ole::*;
use windows::Win32::System::SystemServices::{MK_LBUTTON, MODIFIERKEYS_FLAGS};
use windows::Win32::UI::Shell::Common::ITEMIDLIST;
use windows::Win32::UI::Shell::*;
use windows::Win32::Storage::FileSystem::{FILE_ATTRIBUTE_DIRECTORY, FILE_ATTRIBUTE_NORMAL};
use windows::Win32::System::DataExchange::RegisterClipboardFormatW;

/// Start the drag (the mouse button is still down: the page hands over at its edge). The
/// outcome is posted as files_native_drag_end; Err: it couldn't start.
pub fn drag(v: &Value, post: fn(Value)) -> Result<(), String> {
    ole()?;
    let files = export::selection(v).map_err(|e| e.to_string())?;
    let id = crate::files::scoped_id(v["id"].as_str().unwrap_or(""), &v["window"]);
    let token = crate::files::drag::begin(v, &files).map_err(|e|e.to_string())?;
    // The guard also clears cancelled/failed drags. An accepted destination retains
    // its lease only until its confirmation dialog starts or cancels the transfer.
    struct Finish(String);
    impl Drop for Finish { fn drop(&mut self) { crate::files::drag::finish(&self.0); } }
    let _finish = Finish(token.clone());
    let mut remote_source = None;
    let data: IDataObject = if files.iter().all(|f| !f.remote) { tagged_local(local(&files)?,&token) } else {
        let (data, source)=remote(files, post)?;
        set_marker(&data,&token).map_err(|e|{source.end_batch();e.to_string()})?;
        remote_source=Some(source);
        data
    };
    let mut effect = DROPEFFECT_NONE;
    let hr = unsafe { DoDragDrop(&data, &IDropSource::from(Source), DROPEFFECT_COPY, &mut effect) };
    // Over now, unless Explorer goes on copying in the background (EndOperation ends it).
    if let Some(source) = &remote_source { source.drop_done(); }
    if hr.is_err() {
        return Err(format!("끌어 놓기를 시작할 수 없습니다 ({})", windows::core::Error::from(hr)));
    }
    post(json!({"type":"files_native_drag_end","id":id,"dropped":hr == DRAGDROP_S_DROP && effect != DROPEFFECT_NONE}));
    Ok(())
}

/// DoDragDrop needs OLE (not just COM) on this thread: initialized once, for the thread's
/// life (S_FALSE: it already was, which also counts once).
fn ole() -> Result<(), String> {
    thread_local!(static OLE: std::cell::OnceCell<Result<(), String>> = const { std::cell::OnceCell::new() });
    OLE.with(|o| o.get_or_init(|| unsafe { OleInitialize(None) }.map_err(|e| format!("OLE을 시작할 수 없습니다 ({e})"))).clone())
}

// ---------------------------------------------------------------- this PC's files

/// The shell's data object for the originals (what Explorer itself would drag).
fn local(files: &[export::File]) -> Result<IDataObject, String> {
    let paths: Vec<_> = files.iter().map(|f| f.path().map_err(|e| e.to_string())).collect::<Result<_, _>>()?;
    unsafe {
        let mut pidls = Vec::new();
        for p in &paths {
            let w: Vec<u16> = p.as_os_str().to_string_lossy().encode_utf16().chain([0]).collect();
            let mut pidl: *mut ITEMIDLIST = std::ptr::null_mut();
            if let Err(e) = SHParseDisplayName(PCWSTR(w.as_ptr()), None, &mut pidl, 0, None) {
                pidls.iter().for_each(|&p| ILFree(Some(p)));
                return Err(format!("{}: {e}", p.display()));
            }
            pidls.push(pidl as *const ITEMIDLIST);
        }
        // All from one folder: the first's parent, and each item's last part relative to it.
        let parent = ILClone(pidls[0]);
        let _ = ILRemoveLastID(Some(parent));
        let children: Vec<*const ITEMIDLIST> = pidls.iter().map(|&p| ILFindLastID(p) as *const ITEMIDLIST).collect();
        let r = SHCreateDataObject::<_, IDataObject>(Some(parent), Some(&children), None).map_err(|e| e.to_string());
        ILFree(Some(parent));
        pidls.iter().for_each(|&p| ILFree(Some(p)));
        r
    }
}

// ---------------------------------------------------------------- remote files

fn remote(files: Vec<export::File>, post: fn(Value)) -> Result<(IDataObject, Arc<vdrag::Remote>), String> {
    let source = vdrag::Remote::of(&files).map_err(|e| e.to_string())?;
    let id = files[0].id.clone();
    let kept = source.clone();
    let data = in_mta(move || Virtual::new(files, source, id, post).into()).map_err(|e| { kept.end_batch(); e.to_string() })?;
    Ok((data, kept))
}

/// Make a data object in the multithreaded apartment and hand it to this thread as a proxy.
fn in_mta(make: impl FnOnce() -> IDataObject + Send + 'static) -> windows::core::Result<IDataObject> {
    mta()?;
    let (tx, rx) = std::sync::mpsc::channel::<windows::core::Result<usize>>();
    std::thread::spawn(move || unsafe {
        let r = CoInitializeEx(None, COINIT_MULTITHREADED).ok().and_then(|()| {
            let obj = make();
            let r = CoMarshalInterThreadInterfaceInStream(&IDataObject::IID, &obj).map(|s| s.into_raw() as usize);
            drop(obj);
            CoUninitialize();
            r
        });
        let _ = tx.send(r);
    });
    let raw = rx.recv().map_err(|_| windows::core::Error::from(E_FAIL))??;
    // CoGetInterfaceAndReleaseStream consumes the stream's one reference, success or not:
    // the Rust wrapper must not release it again.
    let stream = std::mem::ManuallyDrop::new(unsafe { IStream::from_raw(raw as *mut _) });
    unsafe { CoGetInterfaceAndReleaseStream::<_, IDataObject>(&*stream) }
}

/// Keep the multithreaded apartment alive (for this process's life) for objects made in it.
fn mta() -> windows::core::Result<()> {
    static COOKIE: OnceLock<Result<usize, HRESULT>> = OnceLock::new();
    match COOKIE.get_or_init(|| unsafe { CoIncrementMTAUsage().map(|c| c.0 as usize).map_err(|e| e.code()) }) {
        Ok(_) => Ok(()),
        Err(hr) => Err((*hr).into()),
    }
}

fn format(name: &str) -> u16 {
    let w: Vec<u16> = name.encode_utf16().chain([0]).collect();
    unsafe { RegisterClipboardFormatW(PCWSTR(w.as_ptr())) as u16 }
}

fn global(bytes: &[u8]) -> windows::core::Result<HGLOBAL> {
    unsafe {
        let h = GlobalAlloc(GMEM_MOVEABLE | GMEM_ZEROINIT, bytes.len().max(1))?;
        let p = GlobalLock(h) as *mut u8;
        if p.is_null() {
            let e = windows::core::Error::from_thread();
            let _ = GlobalFree(Some(h));
            return Err(e);
        }
        std::ptr::copy_nonoverlapping(bytes.as_ptr(), p, bytes.len());
        let _ = GlobalUnlock(h);
        Ok(h)
    }
}

#[implement(IDataObject, IDataObjectAsyncCapability)]
struct Virtual {
    files: Vec<export::File>,
    source: Arc<vdrag::Remote>,
    /// The file list, made when the receiver first asks (listing a folder can take a while).
    plan: Mutex<Option<Result<Vec<vdrag::Item>, String>>>,
    id: String,
    post: fn(Value),
    group: u16,
    contents: u16,
    preferred: u16,
    marker: Mutex<Option<String>>,
    async_mode: AtomicBool,
    in_op: AtomicBool,
}

impl Virtual {
    fn new(files: Vec<export::File>, source: Arc<vdrag::Remote>, id: String, post: fn(Value)) -> Virtual {
        Virtual {
            files,
            source,
            plan: Mutex::new(None),
            id,
            post,
            group: format("FileGroupDescriptorW"),
            contents: format("FileContents"),
            preferred: format("Preferred DropEffect"),
            marker: Mutex::new(None),
            async_mode: AtomicBool::new(true),
            in_op: AtomicBool::new(false),
        }
    }

    fn items(&self) -> Result<Vec<vdrag::Item>, String> {
        let mut g = self.plan.lock().unwrap();
        if g.is_none() {
            let r = self.source.plan(&self.files).map_err(|e| e.to_string());
            if let Err(e) = &r {
                (self.post)(json!({"type":"files_native_drag_end","id":self.id,"error":e}));
            }
            *g = Some(r);
        }
        g.clone().unwrap()
    }

    fn formats(&self) -> Vec<FORMATETC> {
        let f = |cf: u16, tymed: TYMED, lindex: i32| FORMATETC { cfFormat: cf, ptd: std::ptr::null_mut(), dwAspect: DVASPECT_CONTENT.0, lindex, tymed: tymed.0 as u32 };
        let mut formats = vec![f(self.group, TYMED_HGLOBAL, -1), f(self.contents, TYMED_ISTREAM, -1), f(self.preferred, TYMED_HGLOBAL, -1)];
        if self.marker.lock().unwrap().is_some() { formats.push(marker_format()); }
        formats
    }

    fn descriptor(&self, items: &[vdrag::Item]) -> Vec<u8> {
        let mut out = (items.len() as u32).to_ne_bytes().to_vec();
        for i in items {
            let mut d = FILEDESCRIPTORW { dwFlags: (FD_ATTRIBUTES.0 | FD_FILESIZE.0 | FD_WRITESTIME.0 | FD_PROGRESSUI.0 | FD_UNICODE.0) as u32, ..Default::default() };
            d.dwFileAttributes = if i.dir { FILE_ATTRIBUTE_DIRECTORY.0 } else { FILE_ATTRIBUTE_NORMAL.0 };
            (d.nFileSizeHigh, d.nFileSizeLow) = ((i.size >> 32) as u32, i.size as u32);
            // Unix seconds -> FILETIME (100 ns since 1601).
            let t = (i.modified + 11_644_473_600) * 10_000_000;
            d.ftLastWriteTime = FILETIME { dwLowDateTime: t as u32, dwHighDateTime: (t >> 32) as u32 };
            for (k, c) in i.rel.encode_utf16().take(259).enumerate() {
                d.cFileName[k] = c;
            }
            let bytes = unsafe { std::slice::from_raw_parts(&d as *const FILEDESCRIPTORW as *const u8, size_of::<FILEDESCRIPTORW>()) };
            out.extend_from_slice(bytes);
        }
        out
    }
}

impl IDataObject_Impl for Virtual_Impl {
    fn GetData(&self, f: *const FORMATETC) -> windows::core::Result<STGMEDIUM> {
        let f = unsafe { &*f };
        if f.dwAspect != DVASPECT_CONTENT.0 || (f.cfFormat != self.contents && f.lindex != -1) {
            return Err(DV_E_FORMATETC.into());
        }
        let medium = |tymed: TYMED, u: STGMEDIUM_0| STGMEDIUM { tymed: tymed.0 as u32, u, pUnkForRelease: std::mem::ManuallyDrop::new(None) };
        if f.cfFormat == marker_format().cfFormat && f.tymed & TYMED_HGLOBAL.0 as u32 != 0 {
            let marker = self.marker.lock().unwrap();
            let token = marker.as_ref().ok_or(windows::core::Error::from(DV_E_FORMATETC))?;
            return Ok(medium(TYMED_HGLOBAL, STGMEDIUM_0 { hGlobal: global(token.as_bytes())? }));
        }
        if f.cfFormat == self.group && f.tymed & TYMED_HGLOBAL.0 as u32 != 0 {
            let items = self.items().map_err(|_| windows::core::Error::from(E_FAIL))?;
            let h = global(&self.descriptor(&items))?;
            return Ok(medium(TYMED_HGLOBAL, STGMEDIUM_0 { hGlobal: h }));
        }
        if f.cfFormat == self.contents && f.tymed & TYMED_ISTREAM.0 as u32 != 0 {
            let items = self.items().map_err(|_| windows::core::Error::from(E_FAIL))?;
            let item = usize::try_from(f.lindex).ok().and_then(|i| items.get(i)).filter(|i| !i.dir).ok_or(windows::core::Error::from(DV_E_LINDEX))?;
            let s: IStream = Stream { reader: Mutex::new(self.source.reader(item)), size: item.size, at: AtomicU64::new(0) }.into();
            return Ok(medium(TYMED_ISTREAM, STGMEDIUM_0 { pstm: std::mem::ManuallyDrop::new(Some(s)) }));
        }
        if f.cfFormat == self.preferred && f.tymed & TYMED_HGLOBAL.0 as u32 != 0 {
            let h = global(&DROPEFFECT_COPY.0.to_ne_bytes())?;
            return Ok(medium(TYMED_HGLOBAL, STGMEDIUM_0 { hGlobal: h }));
        }
        Err(DV_E_FORMATETC.into())
    }

    fn GetDataHere(&self, _: *const FORMATETC, _: *mut STGMEDIUM) -> windows::core::Result<()> {
        Err(E_NOTIMPL.into())
    }

    fn QueryGetData(&self, f: *const FORMATETC) -> HRESULT {
        let f = unsafe { &*f };
        if f.dwAspect != DVASPECT_CONTENT.0 || (f.cfFormat != self.contents && f.lindex != -1) {
            return DV_E_FORMATETC;
        }
        match self.formats().iter().find(|x| x.cfFormat == f.cfFormat) {
            Some(x) if x.tymed & f.tymed != 0 => S_OK,
            Some(_) => DV_E_TYMED,
            None => DV_E_FORMATETC,
        }
    }

    fn GetCanonicalFormatEtc(&self, _: *const FORMATETC, out: *mut FORMATETC) -> HRESULT {
        if !out.is_null() {
            unsafe { (*out).ptd = std::ptr::null_mut() };
        }
        DATA_S_SAMEFORMATETC
    }

    fn SetData(&self, f: *const FORMATETC, m: *const STGMEDIUM, release: BOOL) -> windows::core::Result<()> {
        if !f.is_null() && !m.is_null() && unsafe { (*f).cfFormat } == marker_format().cfFormat {
            let token = marker_medium(unsafe { &*m });
            if release.as_bool() { unsafe { ReleaseStgMedium(m as *mut STGMEDIUM); } }
            *self.marker.lock().unwrap() = Some(token.ok_or(windows::core::Error::from(DV_E_FORMATETC))?);
            return Ok(());
        }
        // Receivers report back ("Performed DropEffect" and the like); nothing to keep.
        if release.as_bool() && !m.is_null() {
            unsafe { ReleaseStgMedium(m as *mut STGMEDIUM) };
        }
        Ok(())
    }

    fn EnumFormatEtc(&self, dir: u32) -> windows::core::Result<IEnumFORMATETC> {
        if dir != DATADIR_GET.0 as u32 {
            return Err(E_NOTIMPL.into());
        }
        unsafe { SHCreateStdEnumFmtEtc(&self.formats()) }
    }

    fn DAdvise(&self, _: *const FORMATETC, _: u32, _: Ref<IAdviseSink>) -> windows::core::Result<u32> {
        Err(OLE_E_ADVISENOTSUPPORTED.into())
    }

    fn DUnadvise(&self, _: u32) -> windows::core::Result<()> {
        Err(OLE_E_ADVISENOTSUPPORTED.into())
    }

    fn EnumDAdvise(&self) -> windows::core::Result<IEnumSTATDATA> {
        Err(OLE_E_ADVISENOTSUPPORTED.into())
    }
}

/// Lets Explorer copy on its own background thread (with its progress window) instead of
/// inside the drop.
impl IDataObjectAsyncCapability_Impl for Virtual_Impl {
    fn SetAsyncMode(&self, on: BOOL) -> windows::core::Result<()> {
        self.async_mode.store(on.as_bool(), Relaxed);
        Ok(())
    }
    fn GetAsyncMode(&self) -> windows::core::Result<BOOL> {
        Ok(self.async_mode.load(Relaxed).into())
    }
    fn StartOperation(&self, _: Ref<IBindCtx>) -> windows::core::Result<()> {
        self.in_op.store(true, Relaxed);
        // The drag stays running (for quitting) until Explorer's copy ends.
        self.source.start_operation();
        Ok(())
    }
    fn InOperation(&self) -> windows::core::Result<BOOL> {
        Ok(self.in_op.load(Relaxed).into())
    }
    fn EndOperation(&self, hr: HRESULT, _: Ref<IBindCtx>, _: u32) -> windows::core::Result<()> {
        self.in_op.store(false, Relaxed);
        if hr.is_err() {
            self.source.stop();
        }
        self.source.end_operation();
        Ok(())
    }
}

impl Drop for Virtual {
    fn drop(&mut self) {
        self.source.stop();
        self.source.end_batch();
    }
}

/// One remote file's bytes, read front to back.
#[implement(IStream)]
struct Stream {
    reader: Mutex<vdrag::Reader>,
    size: u64,
    at: AtomicU64,
}

impl ISequentialStream_Impl for Stream_Impl {
    fn Read(&self, pv: *mut core::ffi::c_void, cb: u32, read: *mut u32) -> HRESULT {
        if !read.is_null() {
            unsafe { *read = 0 };
        }
        if cb == 0 {
            return S_OK;
        }
        if pv.is_null() {
            return STG_E_INVALIDPOINTER;
        }
        let out = unsafe { std::slice::from_raw_parts_mut(pv as *mut u8, cb as usize) };
        let mut r = self.reader.lock().unwrap();
        let mut n = 0;
        while n < out.len() {
            match r.read(&mut out[n..]) {
                Ok(0) => break,
                Ok(k) => n += k,
                Err(e) => {
                    crate::log!("drag out: read failed: {e}");
                    // What was read still counts, so the position stays right.
                    self.at.fetch_add(n as u64, Relaxed);
                    if !read.is_null() {
                        unsafe { *read = n as u32 };
                    }
                    return E_FAIL;
                }
            }
        }
        self.at.fetch_add(n as u64, Relaxed);
        if !read.is_null() {
            unsafe { *read = n as u32 };
        }
        if n < out.len() { S_FALSE } else { S_OK }
    }
    fn Write(&self, _: *const core::ffi::c_void, _: u32, _: *mut u32) -> HRESULT {
        STG_E_ACCESSDENIED
    }
}

impl IStream_Impl for Stream_Impl {
    fn Seek(&self, mv: i64, origin: STREAM_SEEK, new: *mut u64) -> windows::core::Result<()> {
        // Only "where am I" (and a seek to there) — the bytes arrive in order.
        let at = self.at.load(Relaxed);
        let target = match origin {
            STREAM_SEEK_CUR => at as i64 + mv,
            STREAM_SEEK_SET => mv,
            STREAM_SEEK_END => self.size as i64 + mv,
            _ => -1,
        };
        if target != at as i64 {
            return Err(STG_E_INVALIDFUNCTION.into());
        }
        if !new.is_null() {
            unsafe { *new = at };
        }
        Ok(())
    }
    fn SetSize(&self, _: u64) -> windows::core::Result<()> {
        Err(STG_E_ACCESSDENIED.into())
    }
    fn CopyTo(&self, to: Ref<IStream>, cb: u64, read: *mut u64, written: *mut u64) -> windows::core::Result<()> {
        let to = to.ok()?;
        let mut buf = vec![0u8; 256 * 1024];
        let (mut total_r, mut total_w) = (0u64, 0u64);
        while total_r < cb {
            let want = (cb - total_r).min(buf.len() as u64) as u32;
            let mut got = 0u32;
            let hr = ISequentialStream_Impl::Read(self, buf.as_mut_ptr() as _, want, &mut got);
            hr.ok()?;
            if got == 0 {
                break;
            }
            total_r += got as u64;
            // A destination may take less than offered: write the rest, and fail rather
            // than report a copy whose bytes didn't all arrive.
            let mut off = 0usize;
            while off < got as usize {
                let mut put = 0u32;
                unsafe { to.Write(buf[off..].as_ptr() as _, got - off as u32, Some(&mut put)) }.ok()?;
                if put == 0 {
                    return Err(STG_E_WRITEFAULT.into());
                }
                off += put as usize;
                total_w += put as u64;
            }
        }
        unsafe {
            if !read.is_null() {
                *read = total_r;
            }
            if !written.is_null() {
                *written = total_w;
            }
        }
        Ok(())
    }
    fn Commit(&self, _: &STGC) -> windows::core::Result<()> {
        Ok(())
    }
    fn Revert(&self) -> windows::core::Result<()> {
        Err(E_NOTIMPL.into())
    }
    fn LockRegion(&self, _: u64, _: u64, _: &LOCKTYPE) -> windows::core::Result<()> {
        Err(STG_E_INVALIDFUNCTION.into())
    }
    fn UnlockRegion(&self, _: u64, _: u64, _: u32) -> windows::core::Result<()> {
        Err(STG_E_INVALIDFUNCTION.into())
    }
    fn Stat(&self, st: *mut STATSTG, _: &STATFLAG) -> windows::core::Result<()> {
        if st.is_null() {
            return Err(E_POINTER.into());
        }
        unsafe {
            *st = STATSTG { r#type: STGTY_STREAM.0 as u32, cbSize: self.size, grfMode: STGM_READ, ..Default::default() };
        }
        Ok(())
    }
    fn Clone(&self) -> windows::core::Result<IStream> {
        Err(E_NOTIMPL.into())
    }
}

// ---------------------------------------------------------------- drag source

#[implement(IDropSource)]
struct Source;

impl IDropSource_Impl for Source_Impl {
    fn QueryContinueDrag(&self, escape: BOOL, keys: MODIFIERKEYS_FLAGS) -> HRESULT {
        if escape.as_bool() {
            DRAGDROP_S_CANCEL
        } else if keys.0 & MK_LBUTTON.0 == 0 {
            DRAGDROP_S_DROP
        } else {
            S_OK
        }
    }
    fn GiveFeedback(&self, _: DROPEFFECT) -> HRESULT {
        DRAGDROP_S_USEDEFAULTCURSORS
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// What Explorer does with a remote drag: read the list, then each file's stream —
    /// through the cross-apartment proxy, as a receiver in another process would.
    #[test]
    fn virtual_files_read_like_explorer() {
        let Some(source) = vdrag::test_server() else { return };
        unsafe { OleInitialize(None).ok().unwrap() };
        let root = std::env::temp_dir().join(format!("itda-vdrag-win-{}", crate::store::new_id()));
        std::fs::create_dir_all(root.join("Folder")).unwrap();
        let big: Vec<u8> = (0..(1 << 20) + 5).map(|i| (i % 249) as u8).collect();
        std::fs::write(root.join("Folder").join("big.bin"), &big).unwrap();
        std::fs::write(root.join("한글.txt"), b"hello").unwrap();
        // sftp-server takes /C:/... paths on Windows.
        let src = format!("/{}", root.to_string_lossy().replace('\\', "/"));
        let file = |n: &str, dir: bool| export::File { id: String::new(), remote: true, source: src.clone(), name: n.into(), dir };
        let files = vec![file("Folder", true), file("한글.txt", false)];
        let remote = source.clone();
        let data = in_mta(move || Virtual::new(files, source, String::new(), |_| {}).into()).unwrap();
        set_marker(&data,"0123456789abcdef0123456789abcdef").unwrap();
        assert_eq!(marker(&data).as_deref(),Some("0123456789abcdef0123456789abcdef"));
        assert_eq!(unsafe {data.QueryGetData(&marker_format())},S_OK);
        let fmt = |cf: u16, tymed: TYMED, lindex: i32| FORMATETC { cfFormat: cf, ptd: std::ptr::null_mut(), dwAspect: DVASPECT_CONTENT.0, lindex, tymed: tymed.0 as u32 };
        unsafe {
            let mut m = data.GetData(&fmt(format("FileGroupDescriptorW"), TYMED_HGLOBAL, -1)).unwrap();
            let p = GlobalLock(m.u.hGlobal) as *const FILEGROUPDESCRIPTORW;
            let n = (*p).cItems as usize;
            let ds = std::slice::from_raw_parts((*p).fgd.as_ptr(), n);
            let names: Vec<String> = ds.iter().map(|d| String::from_utf16_lossy(&{ d.cFileName }).trim_end_matches('\0').to_string()).collect();
            assert_eq!(names, ["Folder", "Folder\\big.bin", "한글.txt"]);
            assert!(ds[0].dwFileAttributes & FILE_ATTRIBUTE_DIRECTORY.0 != 0);
            assert_eq!(ds[1].nFileSizeLow as usize, big.len());
            let _ = GlobalUnlock(m.u.hGlobal);
            ReleaseStgMedium(&mut m);
            for (index, want) in [(1, &big[..]), (2, &b"hello"[..])] {
                let mut m = data.GetData(&fmt(format("FileContents"), TYMED_ISTREAM, index)).unwrap();
                let s = (*m.u.pstm).clone().unwrap();
                let mut got = Vec::new();
                let mut buf = vec![0u8; 64 * 1024];
                loop {
                    let mut n = 0u32;
                    let _ = s.Read(buf.as_mut_ptr() as _, buf.len() as u32, Some(&mut n));
                    if n == 0 {
                        break;
                    }
                    got.extend_from_slice(&buf[..n as usize]);
                }
                assert_eq!(got, want);
                ReleaseStgMedium(&mut m);
            }
            assert_eq!(data.QueryGetData(&fmt(format("FileContents"), TYMED_ISTREAM, -1)), S_OK);
            assert_eq!(data.QueryGetData(&fmt(CF_HDROP.0, TYMED_HGLOBAL, -1)), DV_E_FORMATETC);
        }
        // Only "copy" is ever offered.
        drop(data);
        // A stream ends at its last advertised byte, possibly before its download thread has
        // closed the remote file; sftp-server holds it open (no delete sharing) until then.
        assert!(remote.finish(std::time::Duration::from_secs(20)), "a download thread did not finish");
        std::fs::remove_dir_all(root).unwrap();
    }

    fn stream(bytes: &[u8]) -> IStream {
        Stream { reader: Mutex::new(vdrag::Reader::of(bytes, 7)), size: bytes.len() as u64, at: AtomicU64::new(0) }.into()
    }

    #[test]
    fn stream_read_contract() {
        let data: Vec<u8> = (0..100u8).collect();
        let s = stream(&data);
        unsafe {
            let mut n = 99u32;
            assert_eq!(s.Read(std::ptr::null_mut(), 0, Some(&mut n)), S_OK);
            assert_eq!(n, 0);
            assert_eq!(s.Read(std::ptr::null_mut(), 5, Some(&mut n)), STG_E_INVALIDPOINTER);
            let mut buf = [0u8; 60];
            assert_eq!(s.Read(buf.as_mut_ptr() as _, 60, Some(&mut n)), S_OK);
            assert_eq!((n, &buf[..]), (60, &data[..60]));
            assert_eq!(s.Read(buf.as_mut_ptr() as _, 60, Some(&mut n)), S_FALSE);
            assert_eq!((n, &buf[..40]), (40, &data[60..]));
            assert_eq!(s.Read(buf.as_mut_ptr() as _, 60, Some(&mut n)), S_FALSE);
            assert_eq!(n, 0);
            let mut at = 0u64;
            s.Seek(0, STREAM_SEEK_CUR, Some(&mut at)).unwrap();
            assert_eq!(at, 100);
            assert!(s.Seek(0, STREAM_SEEK_SET, None).is_err(), "no rewinding a forward-only stream");
        }
    }

    /// A destination that takes at most `max` bytes per Write (0: none).
    #[implement(IStream)]
    struct Sink {
        got: Mutex<Vec<u8>>,
        max: usize,
    }
    impl ISequentialStream_Impl for Sink_Impl {
        fn Read(&self, _: *mut core::ffi::c_void, _: u32, _: *mut u32) -> HRESULT {
            E_NOTIMPL
        }
        fn Write(&self, pv: *const core::ffi::c_void, cb: u32, done: *mut u32) -> HRESULT {
            let n = (cb as usize).min(self.max);
            self.got.lock().unwrap().extend_from_slice(unsafe { std::slice::from_raw_parts(pv as *const u8, n) });
            unsafe { *done = n as u32 };
            S_OK
        }
    }
    impl IStream_Impl for Sink_Impl {
        fn Seek(&self, _: i64, _: STREAM_SEEK, _: *mut u64) -> windows::core::Result<()> { Err(E_NOTIMPL.into()) }
        fn SetSize(&self, _: u64) -> windows::core::Result<()> { Err(E_NOTIMPL.into()) }
        fn CopyTo(&self, _: Ref<IStream>, _: u64, _: *mut u64, _: *mut u64) -> windows::core::Result<()> { Err(E_NOTIMPL.into()) }
        fn Commit(&self, _: &STGC) -> windows::core::Result<()> { Ok(()) }
        fn Revert(&self) -> windows::core::Result<()> { Err(E_NOTIMPL.into()) }
        fn LockRegion(&self, _: u64, _: u64, _: &LOCKTYPE) -> windows::core::Result<()> { Err(E_NOTIMPL.into()) }
        fn UnlockRegion(&self, _: u64, _: u64, _: u32) -> windows::core::Result<()> { Err(E_NOTIMPL.into()) }
        fn Stat(&self, _: *mut STATSTG, _: &STATFLAG) -> windows::core::Result<()> { Err(E_NOTIMPL.into()) }
        fn Clone(&self) -> windows::core::Result<IStream> { Err(E_NOTIMPL.into()) }
    }

    #[test]
    fn copy_to_writes_everything_or_fails() {
        let data: Vec<u8> = (0..300_000u32).map(|i| (i % 253) as u8).collect();
        // Short writes: every byte still arrives, and the counts agree.
        use windows::core::ComObjectInner;
        let sink = Sink { got: Mutex::new(Vec::new()), max: 1000 }.into_object();
        let to: IStream = sink.to_interface();
        let (mut r, mut w) = (0u64, 0u64);
        unsafe { stream(&data).CopyTo(&to, u64::MAX, Some(&mut r), Some(&mut w)) }.unwrap();
        assert_eq!((r, w), (data.len() as u64, data.len() as u64));
        assert_eq!(*sink.got.lock().unwrap(), data);
        // A destination that takes nothing: an error, not a "successful" empty copy.
        let to: IStream = Sink { got: Mutex::new(Vec::new()), max: 0 }.into();
        assert!(unsafe { stream(&data).CopyTo(&to, u64::MAX, None, None) }.is_err());
    }

    /// Files on this PC go as the shell's own data object, with the originals' paths.
    #[test]
    fn local_files_are_the_originals() {
        unsafe { OleInitialize(None).ok().unwrap() };
        let root = std::env::temp_dir().join(format!("itda-drag-local-{}", crate::store::new_id()));
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join("a.txt"), b"a").unwrap();
        std::fs::write(root.join("b c.txt"), b"b").unwrap();
        let file = |n: &str| export::File { id: String::new(), remote: false, source: root.to_string_lossy().into_owned(), name: n.into(), dir: false };
        let data = local(&[file("a.txt"), file("b c.txt")]).unwrap();
        let f = FORMATETC { cfFormat: CF_HDROP.0, ptd: std::ptr::null_mut(), dwAspect: DVASPECT_CONTENT.0, lindex: -1, tymed: TYMED_HGLOBAL.0 as u32 };
        unsafe {
            let mut m = data.GetData(&f).unwrap();
            let drop = HDROP(m.u.hGlobal.0);
            let n = DragQueryFileW(drop, u32::MAX, None);
            let paths: Vec<String> = (0..n).map(|i| {
                let mut b = [0u16; 520];
                let k = DragQueryFileW(drop, i, Some(&mut b));
                String::from_utf16_lossy(&b[..k as usize])
            }).collect();
            ReleaseStgMedium(&mut m);
            // (The temp folder may be given in its 8.3 form; compare the files themselves.)
            let real = |p: &str| std::fs::canonicalize(p).unwrap();
            let got: Vec<_> = paths.iter().map(|p| real(p)).collect();
            let want: Vec<_> = ["a.txt", "b c.txt"].iter().map(|n| real(&root.join(n).to_string_lossy())).collect();
            assert_eq!(got, want);
        }
        std::fs::remove_dir_all(root).unwrap();
    }
}

// Shell data objects can return E_NOTIMPL from SetData. Wrap their IDataObject
// interface instead; every original format stays owned/implemented by the shell.
#[implement(IDataObject)]
struct TaggedLocal {
    native: IDataObject,
    token: String,
}
fn tagged_local(native: IDataObject, token: &str) -> IDataObject {
    TaggedLocal {
        native,
        token: token.into(),
    }
    .into()
}
impl IDataObject_Impl for TaggedLocal_Impl {
    fn GetData(&self, f: *const FORMATETC) -> windows::core::Result<STGMEDIUM> {
        if !f.is_null() && unsafe { (*f).cfFormat } == marker_format().cfFormat {
            self.QueryGetData(f).ok()?;
            return Ok(STGMEDIUM {
                tymed: TYMED_HGLOBAL.0 as u32,
                u: STGMEDIUM_0 {
                    hGlobal: global(self.token.as_bytes())?,
                },
                pUnkForRelease: std::mem::ManuallyDrop::new(None),
            });
        }
        unsafe { self.native.GetData(f) }
    }
    fn GetDataHere(&self, f: *const FORMATETC, m: *mut STGMEDIUM) -> windows::core::Result<()> {
        unsafe { self.native.GetDataHere(f, m) }
    }
    fn QueryGetData(&self, f: *const FORMATETC) -> HRESULT {
        if !f.is_null() && unsafe { (*f).cfFormat } == marker_format().cfFormat {
            let f = unsafe { &*f };
            if f.dwAspect != DVASPECT_CONTENT.0 || f.lindex != -1 {
                DV_E_FORMATETC
            } else if f.tymed & TYMED_HGLOBAL.0 as u32 == 0 {
                DV_E_TYMED
            } else {
                S_OK
            }
        } else {
            unsafe { self.native.QueryGetData(f) }
        }
    }
    fn GetCanonicalFormatEtc(&self, f: *const FORMATETC, out: *mut FORMATETC) -> HRESULT {
        if !f.is_null() && unsafe { (*f).cfFormat } == marker_format().cfFormat {
            if out.is_null() {
                return E_POINTER;
            }
            unsafe {
                (*out).ptd = std::ptr::null_mut();
            }
            DATA_S_SAMEFORMATETC
        } else {
            unsafe { self.native.GetCanonicalFormatEtc(f, out) }
        }
    }
    fn SetData(
        &self,
        f: *const FORMATETC,
        m: *const STGMEDIUM,
        release: BOOL,
    ) -> windows::core::Result<()> {
        unsafe { self.native.SetData(f, m, release.as_bool()) }
    }
    fn EnumFormatEtc(&self, dir: u32) -> windows::core::Result<IEnumFORMATETC> {
        let native = unsafe { self.native.EnumFormatEtc(dir) }?;
        if dir != DATADIR_GET.0 as u32 {
            return Ok(native);
        }
        Ok(TaggedFormats {
            native,
            first: std::cell::Cell::new(true),
        }
        .into())
    }
    fn DAdvise(
        &self,
        f: *const FORMATETC,
        flags: u32,
        sink: Ref<IAdviseSink>,
    ) -> windows::core::Result<u32> {
        unsafe { self.native.DAdvise(f, flags, sink.as_ref()) }
    }
    fn DUnadvise(&self, id: u32) -> windows::core::Result<()> {
        unsafe { self.native.DUnadvise(id) }
    }
    fn EnumDAdvise(&self) -> windows::core::Result<IEnumSTATDATA> {
        unsafe { self.native.EnumDAdvise() }
    }
}
#[implement(IEnumFORMATETC)]
struct TaggedFormats {
    native: IEnumFORMATETC,
    first: std::cell::Cell<bool>,
}
impl IEnumFORMATETC_Impl for TaggedFormats_Impl {
    fn Next(&self, count: u32, out: *mut FORMATETC, got: *mut u32) -> HRESULT {
        if out.is_null() || (count != 1 && got.is_null()) {
            return E_POINTER;
        }
        let mut n = 0;
        if count > 0 && self.first.replace(false) {
            unsafe {
                *out = marker_format();
            }
            n = 1;
        }
        let mut read = 0;
        let status = if n < count {
            unsafe {
                self.native.Next(
                    std::slice::from_raw_parts_mut(out.add(n as usize), (count - n) as usize),
                    Some(&mut read),
                )
            }
        } else {
            S_OK
        };
        n += read;
        if !got.is_null() {
            unsafe {
                *got = n;
            }
        }
        if status.is_err() {
            status
        } else if n == count {
            S_OK
        } else {
            S_FALSE
        }
    }
    fn Skip(&self, mut count: u32) -> windows::core::Result<()> {
        if count > 0 && self.first.replace(false) {
            count -= 1;
        }
        if count == 0 {
            Ok(())
        } else {
            unsafe { self.native.Skip(count) }
        }
    }
    fn Reset(&self) -> windows::core::Result<()> {
        unsafe { self.native.Reset() }?;
        self.first.set(true);
        Ok(())
    }
    fn Clone(&self) -> windows::core::Result<IEnumFORMATETC> {
        Ok(TaggedFormats {
            native: unsafe { self.native.Clone() }?,
            first: std::cell::Cell::new(self.first.get()),
        }
        .into())
    }
}

// ---------------------------------------------------------------- Itda window receivers
fn marker_format() -> FORMATETC {
    FORMATETC {
        cfFormat: format("dev.jiun.itda.file-drag"),
        ptd: std::ptr::null_mut(),
        dwAspect: DVASPECT_CONTENT.0,
        lindex: -1,
        tymed: TYMED_HGLOBAL.0 as u32,
    }
}
fn set_marker(data: &IDataObject, token: &str) -> windows::core::Result<()> {
    let mut medium = STGMEDIUM {
        tymed: TYMED_HGLOBAL.0 as u32,
        u: STGMEDIUM_0 {
            hGlobal: global(token.as_bytes())?,
        },
        pUnkForRelease: std::mem::ManuallyDrop::new(None),
    };
    let result = unsafe { data.SetData(&marker_format(), &medium, false) };
    unsafe {
        ReleaseStgMedium(&mut medium);
    }
    result
}
fn marker_medium(medium: &STGMEDIUM) -> Option<String> {
    if medium.tymed != TYMED_HGLOBAL.0 as u32 {
        return None;
    }
    unsafe {
        let handle = medium.u.hGlobal;
        let len = GlobalSize(handle);
        if !(32..=128).contains(&len) {
            return None;
        }
        let bytes = GlobalLock(handle) as *const u8;
        if bytes.is_null() {
            return None;
        }
        let bytes = std::slice::from_raw_parts(bytes, len);
        let token = std::str::from_utf8(&bytes[..32])
            .ok()
            .filter(|s| s.bytes().all(|b| b.is_ascii_hexdigit()))
            .map(str::to_string);
        let _ = GlobalUnlock(handle);
        token
    }
}
fn marker(data: &IDataObject) -> Option<String> {
    let mut medium = unsafe { data.GetData(&marker_format()) }.ok()?;
    let token = marker_medium(&medium);
    unsafe {
        ReleaseStgMedium(&mut medium);
    }
    token
}
fn path_format() -> FORMATETC {
    FORMATETC {
        cfFormat: CF_HDROP.0,
        ptd: std::ptr::null_mut(),
        dwAspect: DVASPECT_CONTENT.0,
        lindex: -1,
        tymed: TYMED_HGLOBAL.0 as u32,
    }
}
fn drop_paths(data: &IDataObject) -> Option<Vec<std::path::PathBuf>> {
    let mut medium = unsafe { data.GetData(&path_format()) }.ok()?;
    let result = (|| unsafe {
        if medium.tymed != TYMED_HGLOBAL.0 as u32 {
            return None;
        }
        let hdrop = HDROP(medium.u.hGlobal.0);
        let count = DragQueryFileW(hdrop, u32::MAX, None);
        if count == 0 || count > 10_000 {
            return None;
        }
        let mut paths = Vec::new();
        for i in 0..count {
            let n = DragQueryFileW(hdrop, i, None) as usize;
            if n == 0 || n > 32_767 {
                return None;
            }
            let mut path = vec![0; n + 1];
            if DragQueryFileW(hdrop, i, Some(&mut path)) as usize != n {
                return None;
            }
            use std::os::windows::ffi::OsStringExt;
            paths.push(std::ffi::OsString::from_wide(&path[..n]).into());
        }
        Some(paths)
    })();
    // The medium owns the HDROP; DragFinish would free it a second time.
    unsafe {
        ReleaseStgMedium(&mut medium);
    }
    result
}
#[implement(IDropTarget)]
struct WindowDrop {
    root: isize,
    window: Option<String>,
    post: fn(Value),
    valid: std::cell::Cell<bool>,
}
impl WindowDrop {
    fn allowed(&self, data: &IDataObject) -> bool {
        match marker(data) {
            Some(token) => crate::files::drag::valid(&token),
            None => unsafe { data.QueryGetData(&path_format()) }.is_ok(),
        }
    }
    fn position(&self, pt: &POINTL) -> Option<[f64; 2]> {
        let mut point = windows_sys::Win32::Foundation::POINT { x: pt.x, y: pt.y };
        let root = self.root as windows_sys::Win32::Foundation::HWND;
        if unsafe { windows_sys::Win32::Graphics::Gdi::ScreenToClient(root, &mut point) } == 0 {
            return None;
        }
        let dpi = unsafe { windows_sys::Win32::UI::HiDpi::GetDpiForWindow(root) }.max(96) as f64;
        Some([point.x as f64 * 96. / dpi, point.y as f64 * 96. / dpi])
    }
}
impl IDropTarget_Impl for WindowDrop_Impl {
    fn DragEnter(
        &self,
        data: Ref<IDataObject>,
        _: MODIFIERKEYS_FLAGS,
        _: &POINTL,
        effect: *mut DROPEFFECT,
    ) -> windows::core::Result<()> {
        let allowed = data.as_ref().is_some_and(|data| self.allowed(data));
        self.valid.set(allowed);
        unsafe {
            *effect = if allowed && (*effect).0 & DROPEFFECT_COPY.0 != 0 {
                DROPEFFECT_COPY
            } else {
                DROPEFFECT_NONE
            };
        }
        Ok(())
    }
    fn DragOver(
        &self,
        _: MODIFIERKEYS_FLAGS,
        _: &POINTL,
        effect: *mut DROPEFFECT,
    ) -> windows::core::Result<()> {
        unsafe {
            *effect = if self.valid.get() && (*effect).0 & DROPEFFECT_COPY.0 != 0 {
                DROPEFFECT_COPY
            } else {
                DROPEFFECT_NONE
            };
        }
        Ok(())
    }
    fn DragLeave(&self) -> windows::core::Result<()> {
        self.valid.set(false);
        Ok(())
    }
    fn Drop(
        &self,
        data: Ref<IDataObject>,
        _: MODIFIERKEYS_FLAGS,
        pt: &POINTL,
        effect: *mut DROPEFFECT,
    ) -> windows::core::Result<()> {
        let can_copy = unsafe { (*effect).0 & DROPEFFECT_COPY.0 != 0 };
        unsafe {
            *effect = DROPEFFECT_NONE;
        }
        self.valid.set(false);
        let Some(data) = data.as_ref().filter(|_| can_copy) else {
            return Ok(());
        };
        let event = match marker(data) {
            Some(token) => crate::files::drag::accept(&token).map(|origin| json!({"type":"files_drop","window":self.window,"origin":origin,"position":self.position(pt)})),
            None => drop_paths(data).map(|paths|json!({"type":"files_drop","window":self.window,"paths":paths,"position":self.position(pt)})),
        };
        if let Some(event) = event {
            (self.post)(event);
            unsafe {
                *effect = DROPEFFECT_COPY;
            }
        }
        Ok(())
    }
}
struct Receivers {
    root: isize,
    window: Option<String>,
    post: fn(Value),
    targets: Vec<(HWND, IDropTarget)>,
}
impl Drop for Receivers {
    fn drop(&mut self) {
        for (hwnd, _) in &self.targets {
            let _ = unsafe { RevokeDragDrop(*hwnd) };
        }
    }
}
thread_local! { static RECEIVERS:std::cell::RefCell<std::collections::HashMap<isize,Receivers>> = std::cell::RefCell::new(std::collections::HashMap::new()); }
pub fn forget_drop(root: isize) {
    RECEIVERS.with(|r| r.borrow_mut().remove(&root));
}
pub fn install_drop(root: isize, window: Option<String>, post: fn(Value)) -> Result<(), String> {
    ole()?;
    forget_drop(root);
    let mut receivers = Receivers {
        root,
        window,
        post,
        targets: Vec::new(),
    };
    unsafe extern "system" fn each(
        hwnd: windows_sys::Win32::Foundation::HWND,
        param: isize,
    ) -> windows_sys::Win32::Foundation::BOOL {
        let receivers = &mut *(param as *mut Receivers);
        let hwnd = HWND(hwnd);
        if RevokeDragDrop(hwnd) != Err(DRAGDROP_E_INVALIDHWND.into()) {
            let target: IDropTarget = WindowDrop {
                root: receivers.root,
                window: receivers.window.clone(),
                post: receivers.post,
                valid: std::cell::Cell::new(false),
            }
            .into();
            if RegisterDragDrop(hwnd, &target).is_ok() {
                receivers.targets.push((hwnd, target));
            }
        }
        1
    }
    unsafe {
        windows_sys::Win32::UI::WindowsAndMessaging::EnumChildWindows(
            root as _,
            Some(each),
            &mut receivers as *mut Receivers as isize,
        );
    }
    if receivers.targets.is_empty() {
        return Err("파일을 받을 창을 등록하지 못했습니다".into());
    }
    RECEIVERS.with(|r| r.borrow_mut().insert(root, receivers));
    Ok(())
}

#[cfg(test)]
mod receiver_tests {
    use super::*;
    static EVENTS: Mutex<Vec<Value>> = Mutex::new(Vec::new());
    fn post(v: Value) {
        EVENTS.lock().unwrap().push(v);
    }
    #[test]
    fn remote_marker_requires_no_listing_or_download() {
        ole().unwrap();
        let source = vdrag::Remote::new(Box::new(|_| {
            panic!("private drag token requested remote file data")
        }));
        let data = in_mta(move || {
            Virtual::new(
                vec![export::File {
                    id: "source".into(),
                    remote: true,
                    source: "/remote".into(),
                    name: "file.txt".into(),
                    dir: false,
                }],
                source,
                String::new(),
                |_| {},
            )
            .into()
        })
        .unwrap();
        set_marker(&data, "0123456789abcdef0123456789abcdef").unwrap();
        assert_eq!(
            marker(&data).as_deref(),
            Some("0123456789abcdef0123456789abcdef")
        );
    }
    #[test]
    fn native_receiver_checks_provenance_and_keeps_explorer_paths() {
        ole().unwrap();
        let root = std::env::temp_dir().join(format!("itda-drop-{}", crate::store::new_id()));
        std::fs::create_dir(&root).unwrap();
        std::fs::write(root.join("한글.txt"), b"source bytes").unwrap();
        let v = json!({"id":"source","window":"source-window","side":"local","path":root,"items":[{"name":"한글.txt"}]});
        let files = export::selection(&v).unwrap();
        let data = local(&files).unwrap();
        let target: IDropTarget = WindowDrop {
            root: 0,
            window: Some("destination-window".into()),
            post,
            valid: std::cell::Cell::new(false),
        }
        .into();
        let point = POINTL { x: 0, y: 0 };
        // A normal Explorer/shell drop still carries its original paths.
        let mut effect = DROPEFFECT_COPY;
        unsafe { target.DragEnter(&data, MODIFIERKEYS_FLAGS(0), point, &mut effect) }.unwrap();
        assert_eq!(effect, DROPEFFECT_COPY);
        unsafe { target.Drop(&data, MODIFIERKEYS_FLAGS(0), point, &mut effect) }.unwrap();
        let event = EVENTS.lock().unwrap().pop().unwrap();
        assert_eq!(event["window"], "destination-window");
        let path = std::path::PathBuf::from(event["paths"][0].as_str().unwrap());
        assert_eq!(std::fs::read(path).unwrap(), b"source bytes");
        // Itda's private format is accepted once, from a live session only.
        let token = crate::files::drag::begin(&v, &files).unwrap();
        let data = tagged_local(data, &token);
        assert_eq!(marker(&data), Some(token.clone()));
        let original_paths = drop_paths(&data).unwrap();
        assert_eq!(std::fs::read(&original_paths[0]).unwrap(), b"source bytes");
        let formats = unsafe { data.EnumFormatEtc(DATADIR_GET.0 as u32) }.unwrap();
        let mut first = [FORMATETC::default()];
        let mut got = 0;
        assert_eq!(unsafe { formats.Next(&mut first, Some(&mut got)) }, S_OK);
        assert_eq!(got, 1);
        assert_eq!(first[0].cfFormat, marker_format().cfFormat);
        unsafe { formats.Reset() }.unwrap();
        effect = DROPEFFECT_COPY;
        unsafe { target.DragEnter(&data, MODIFIERKEYS_FLAGS(0), point, &mut effect) }.unwrap();
        assert_eq!(effect, DROPEFFECT_COPY);
        unsafe { target.Drop(&data, MODIFIERKEYS_FLAGS(0), point, &mut effect) }.unwrap();
        let event = EVENTS.lock().unwrap().pop().unwrap();
        assert_eq!(event["origin"]["window"], "source-window");
        assert_eq!(event["origin"]["names"], json!(["한글.txt"]));
        crate::files::drag::finish(&token);
        effect = DROPEFFECT_COPY;
        unsafe { target.DragEnter(&data, MODIFIERKEYS_FLAGS(0), point, &mut effect) }.unwrap();
        assert_eq!(effect, DROPEFFECT_NONE);
        crate::files::drag::release(&token);
        let data = tagged_local(data, "00000000000000000000000000000000");
        effect = DROPEFFECT_COPY;
        unsafe { target.DragEnter(&data, MODIFIERKEYS_FLAGS(0), point, &mut effect) }.unwrap();
        assert_eq!(effect, DROPEFFECT_NONE);
        unsafe { target.Drop(&data, MODIFIERKEYS_FLAGS(0), point, &mut effect) }.unwrap();
        assert!(EVENTS.lock().unwrap().is_empty());
        std::fs::remove_dir_all(root).unwrap();
    }
}
