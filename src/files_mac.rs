//! Native Finder clipboard and lazy file drags for the file browser.
use crate::files::{self, export::File};
use objc2::{
    define_class, msg_send,
    rc::Retained,
    runtime::{AnyObject, NSObject, NSObjectProtocol, ProtocolObject},
    AnyThread, DeclaredClass, MainThreadMarker, MainThreadOnly,
};
use objc2_app_kit::{
    NSApp, NSDragOperation, NSDraggingContext, NSDraggingItem, NSDraggingSession, NSDraggingSource,
    NSEvent, NSEventModifierFlags, NSEventType, NSFilePromiseProvider,
    NSFilePromiseProviderDelegate, NSImage, NSPasteboard, NSPasteboardTypeFileURL,
    NSPasteboardWriting, NSView,
};
use objc2_foundation::{
    NSArray, NSData, NSDictionary, NSError, NSLocalizedDescriptionKey, NSOperationQueue, NSPoint,
    NSRect, NSSize, NSString, NSURL,
};
use serde_json::{json, Value};
use std::{
    cell::{Cell, RefCell},
    collections::HashMap,
    path::PathBuf,
    sync::Arc,
};
thread_local! { static PROMISES: RefCell<HashMap<String,Retained<Promise>>> = RefCell::new(HashMap::new()); static CLIP: Cell<(u64,isize)> = const {Cell::new((0,0))}; }
fn emit(v: Value) {
    super::super::post(super::super::Event::Page(v));
}
fn emitter() -> files::Emit {
    Arc::new(emit)
}

pub fn copy(v: Value) {
    let id = v["id"].clone();
    let seq = v["seq"].clone();
    let selection = match files::export::selection(&v) {
        Ok(s) => s,
        Err(e) => {
            emit(
                json!({"type":"files_result","op":"native_copy","id":id,"seq":seq,"ok":false,"error":e.to_string()}),
            );
            return;
        }
    };
    let pb = NSPasteboard::generalPasteboard();
    // Clear stale HTML immediately, so Finder cannot create a .textClipping while a
    // remote selection is being downloaded. Never put a textual path on the board.
    let count = pb.clearContents();
    let serial = CLIP.with(|c| {
        let n = c.get().0.wrapping_add(1);
        c.set((n, count));
        n
    });
    emit(
        json!({"type":"files_result","op":"native_copy_started","id":id,"seq":seq,"ok":true,"data":{"stamp":count}}),
    );
    std::thread::spawn(move || {
        let result = files::export::prepare(&selection, emitter());
        emit(
            json!({"type":"files_native_copy_ready","id":id,"seq":seq,"serial":serial,"ok":result.is_ok(),"data":{"paths":result.as_ref().ok()},"error":result.err().map(|e|e.to_string())}),
        );
    });
}
pub fn finish_copy(v: &mut Value) {
    v["type"] = json!("files_result");
    v["op"] = json!("native_copy");
    if v["ok"] != true {
        return;
    }
    let pb = NSPasteboard::generalPasteboard();
    if !CLIP.with(|c| c.get() == (v["serial"].as_u64().unwrap_or(0), pb.changeCount())) {
        v["ok"] = json!(false);
        v["error"] = json!("다른 내용을 복사해 Finder용 파일 준비를 취소했습니다");
        return;
    }
    let urls: Vec<_> = v["data"]["paths"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(|path| {
            ProtocolObject::<dyn NSPasteboardWriting>::from_retained(NSURL::fileURLWithPath(
                &NSString::from_str(path),
            ))
        })
        .collect();
    if urls.is_empty() || !pb.writeObjects(&NSArray::from_retained_slice(&urls)) {
        v["ok"] = json!(false);
        v["error"] = json!("Finder 파일 클립보드를 만들지 못했습니다");
    } else {
        v["data"]["stamp"] = json!(pb.changeCount());
    }
}
pub fn paste(v: Value) {
    let pb = NSPasteboard::generalPasteboard();
    let internal = v["stamp"].as_i64() == Some(pb.changeCount() as i64);
    let mut paths = Vec::new();
    if !internal {
        if let Some(items) = pb.pasteboardItems() {
            for item in items.iter() {
                if let Some(s) = unsafe { item.stringForType(NSPasteboardTypeFileURL) } {
                    if let Some(url) = NSURL::URLWithString(&s) {
                        if url.isFileURL() {
                            if let Some(path) = url.path() {
                                paths.push(PathBuf::from(path.to_string()));
                            }
                        }
                    }
                }
            }
        }
    }
    emit(
        json!({"type":"files_native_paste","id":v["id"],"internal":internal,"paths":paths,"side":v["side"],"path":v["path"]}),
    );
}

struct PromiseState {
    file: File,
    token: String,
    queue: Retained<NSOperationQueue>,
}
define_class!(
    #[unsafe(super(NSObject))]
    #[name = "ItdaFilePromiseDelegate"]
    #[ivars=PromiseState]
    struct Promise;
    unsafe impl NSObjectProtocol for Promise {}
    unsafe impl NSFilePromiseProviderDelegate for Promise {
        #[unsafe(method_id(filePromiseProvider:fileNameForType:))]
        fn filename(
            &self,
            _provider: &NSFilePromiseProvider,
            _kind: &NSString,
        ) -> Retained<NSString> {
            NSString::from_str(&self.ivars().file.name)
        }
        #[unsafe(method_id(operationQueueForFilePromiseProvider:))]
        fn queue(&self, _provider: &NSFilePromiseProvider) -> Retained<NSOperationQueue> {
            self.ivars().queue.clone()
        }
        #[unsafe(method(filePromiseProvider:writePromiseToURL:completionHandler:))]
        fn write(
            &self,
            _provider: &NSFilePromiseProvider,
            url: &NSURL,
            done: &block2::DynBlock<dyn Fn(*mut NSError)>,
        ) {
            let result = if url.isFileURL() { url.path() } else { None }
                .ok_or_else(|| std::io::Error::other("Finder 복사 위치를 확인할 수 없습니다"))
                .and_then(|path| {
                    files::export::write(
                        &self.ivars().file,
                        &PathBuf::from(path.to_string()),
                        emitter(),
                    )
                });
            let err = result.err().map(|e| {
                let message = NSString::from_str(&e.to_string());
                let value: &AnyObject = &message;
                let info =
                    NSDictionary::from_slices(&[unsafe { NSLocalizedDescriptionKey }], &[value]);
                unsafe {
                    NSError::errorWithDomain_code_userInfo(
                        &NSString::from_str("ItdaFileTransfer"),
                        1,
                        Some(&info),
                    )
                }
            });
            done.call((err.as_ref().map_or(std::ptr::null_mut(), |e| {
                Retained::as_ptr(e) as *mut NSError
            }),));
            emit(json!({"type":"files_native_promise_done","token":self.ivars().token}));
        }
    }
);
struct DragState {
    tokens: Vec<String>,
    id: String,
}
define_class!(
    #[unsafe(super(NSObject))]
    #[thread_kind=MainThreadOnly]
    #[name = "ItdaFileDraggingSource"]
    #[ivars=DragState]
    struct Source;
    unsafe impl NSObjectProtocol for Source {}
    unsafe impl NSDraggingSource for Source {
        #[unsafe(method(draggingSession:sourceOperationMaskForDraggingContext:))]
        fn operation(
            &self,
            _session: &NSDraggingSession,
            _context: NSDraggingContext,
        ) -> NSDragOperation {
            NSDragOperation::Copy
        }
        #[unsafe(method(draggingSession:endedAtPoint:operation:))]
        fn ended(&self, _session: &NSDraggingSession, _point: NSPoint, operation: NSDragOperation) {
            if operation == NSDragOperation::None {
                let _ = PROMISES.try_with(|p| {
                    let mut p = p.borrow_mut();
                    for token in &self.ivars().tokens {
                        p.remove(token);
                    }
                });
            }
            emit(
                json!({"type":"files_native_drag_end","id":self.ivars().id,"dropped":operation!=NSDragOperation::None}),
            );
        }
    }
);
/// Begin immediately while the button is held. Remote bytes are copied only after
/// Finder accepts the drop, on an NSOperationQueue, directly into its chosen URL.
pub fn drag(nsview: &NSView, v: Value) -> Result<(), String> {
    let selection = files::export::selection(&v).map_err(|e| e.to_string())?;
    let mtm = MainThreadMarker::new().ok_or("파일 드래그는 UI 스레드에서 시작해야 합니다")?;
    if NSEvent::pressedMouseButtons() & 1 == 0 {
        return Err("파일을 누른 채 Finder로 끌어 주세요".into());
    }
    let window = nsview.window().ok_or("파일 창을 찾을 수 없습니다")?;
    let content = window.contentView().ok_or("파일 창을 찾을 수 없습니다")?;
    let position = window.mouseLocationOutsideOfEventStream();
    let image = NSImage::initWithData(
        NSImage::alloc(),
        &NSData::with_bytes(include_bytes!("../assets/itda-192.png")),
    )
    .ok_or("드래그 이미지를 만들지 못했습니다")?;
    image.setSize(NSSize::new(40., 40.));
    let frame = NSRect::new(
        NSPoint::new(position.x - 20., position.y - 20.),
        NSSize::new(40., 40.),
    );
    if selection.len() > 256 {
        return Err(
            "한 번에 256개까지 Finder로 끌어 놓을 수 있습니다. 폴더로 묶어 다시 시도하세요".into(),
        );
    }
    let queue = NSOperationQueue::new();
    queue.setMaxConcurrentOperationCount(2);
    let mut tokens = Vec::new();
    let mut items = Vec::new();
    for file in selection {
        let writer: Retained<ProtocolObject<dyn NSPasteboardWriting>> = if file.remote {
            let token = crate::store::new_id();
            let object = Promise::alloc().set_ivars(PromiseState {
                file: file.clone(),
                token: token.clone(),
                queue: queue.clone(),
            });
            let delegate: Retained<Promise> = unsafe { msg_send![super(object), init] };
            let provider = NSFilePromiseProvider::initWithFileType_delegate(
                NSFilePromiseProvider::alloc(),
                &NSString::from_str(if file.dir {
                    "public.folder"
                } else {
                    "public.data"
                }),
                ProtocolObject::from_ref(&*delegate),
            );
            PROMISES.with(|p| {
                p.borrow_mut().insert(token.clone(), delegate);
            });
            tokens.push(token);
            ProtocolObject::from_retained(provider)
        } else {
            let path = file.path().map_err(|e| e.to_string())?;
            std::fs::symlink_metadata(&path).map_err(|e| e.to_string())?;
            ProtocolObject::from_retained(NSURL::fileURLWithPath(&NSString::from_str(
                &path.to_string_lossy(),
            )))
        };
        let item = NSDraggingItem::initWithPasteboardWriter(NSDraggingItem::alloc(), &writer);
        unsafe { item.setDraggingFrame_contents(frame, Some(&image)) };
        items.push(item);
    }
    let state = Source::alloc(mtm).set_ivars(DragState {
        tokens,
        id: v["id"].as_str().unwrap_or("").into(),
    });
    let source: Retained<Source> = unsafe { msg_send![super(state), init] };
    let timestamp = NSApp(mtm).currentEvent().map_or(0., |e| e.timestamp());
    let event=NSEvent::mouseEventWithType_location_modifierFlags_timestamp_windowNumber_context_eventNumber_clickCount_pressure(NSEventType::LeftMouseDragged,position,NSEventModifierFlags::empty(),timestamp,window.windowNumber(),None,0,1,1.).ok_or("드래그 이벤트를 만들지 못했습니다")?;
    content.beginDraggingSessionWithItems_event_source(
        &NSArray::from_retained_slice(&items),
        &event,
        ProtocolObject::from_ref(&*source),
    );
    Ok(())
}

pub fn promise_done(v: &Value) {
    if let Some(token) = v["token"].as_str() {
        PROMISES.with(|p| {
            p.borrow_mut().remove(token);
        });
    }
}
pub fn shutdown() {
    let promises = PROMISES.with(|p| std::mem::take(&mut *p.borrow_mut()));
    drop(promises);
}
pub fn system_edit(action: &str) {
    use objc2::sel;
    if let Some(mtm) = MainThreadMarker::new() {
        let selector = match action {
            "copy" => sel!(copy:),
            "cut" => sel!(cut:),
            "paste" => sel!(paste:),
            _ => return,
        };
        unsafe { NSApp(mtm).sendAction_to_from(selector, None, None) };
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;
    #[test]
    fn finder_promise_writes_and_never_overwrites_or_removes_source() {
        let root =
            std::env::temp_dir().join(format!("itda-promise-test-{}", crate::store::new_id()));
        std::fs::create_dir(&root).unwrap();
        std::fs::create_dir(root.join("source")).unwrap();
        std::fs::write(root.join("source/한글 문서.txt"), b"file promise bytes").unwrap();
        let file = File {
            id: "promise-test".into(),
            remote: false,
            source: root.to_string_lossy().into_owned(),
            name: "source".into(),
            dir: true,
        };
        let queue = NSOperationQueue::new();
        let object = Promise::alloc().set_ivars(PromiseState {
            file,
            token: "test".into(),
            queue,
        });
        let delegate: Retained<Promise> = unsafe { msg_send![super(object), init] };
        let protocol = ProtocolObject::<dyn NSFilePromiseProviderDelegate>::from_ref(&*delegate);
        let provider = NSFilePromiseProvider::initWithFileType_delegate(
            NSFilePromiseProvider::alloc(),
            &NSString::from_str("public.folder"),
            protocol,
        );
        let target = root.join("Finder copy");
        let url = NSURL::fileURLWithPath(&NSString::from_str(&target.to_string_lossy()));
        let errors = Arc::new(Mutex::new(Vec::new()));
        let recorded = errors.clone();
        let done = block2::RcBlock::new(move |error: *mut NSError| {
            recorded.lock().unwrap().push(!error.is_null());
        });
        protocol.filePromiseProvider_writePromiseToURL_completionHandler(&provider, &url, &done);
        assert_eq!(*errors.lock().unwrap(), [false]);
        assert_eq!(
            std::fs::read(target.join("한글 문서.txt")).unwrap(),
            b"file promise bytes"
        );
        assert_eq!(
            std::fs::read(root.join("source/한글 문서.txt")).unwrap(),
            b"file promise bytes"
        );
        std::fs::write(target.join("한글 문서.txt"), b"keep destination").unwrap();
        protocol.filePromiseProvider_writePromiseToURL_completionHandler(&provider, &url, &done);
        assert_eq!(*errors.lock().unwrap(), [false, true]);
        assert_eq!(
            std::fs::read(target.join("한글 문서.txt")).unwrap(),
            b"keep destination"
        );
        std::fs::remove_dir_all(root).unwrap();
    }
}
