//! Native Finder clipboard and lazy file drags for the file browser.
use crate::files::{self, export::File};
use objc2::{
    define_class, msg_send,
    rc::Retained,
    runtime::{AnyObject, NSObject, NSObjectProtocol, ProtocolObject},
    AnyThread, DeclaredClass, MainThreadMarker, MainThreadOnly, Message,
};
use objc2_app_kit::{
    NSApp, NSDragOperation, NSDraggingContext, NSDraggingItem, NSDraggingSession, NSDraggingSource,
    NSEvent, NSEventModifierFlags, NSEventType, NSFilePromiseProvider,
    NSFilePromiseProviderDelegate, NSImage, NSPasteboard, NSPasteboardTypeFileURL,
    NSPasteboardWriting, NSView, NSPasteboardNameDrag, NSPasteboardWritingOptions,
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
/// Each Finder drag's promise queue, while the drag can still be fulfilled: quitting asks
/// while any has operations queued or running (an idle promise Finder never asked for has
/// none). NSOperationQueue is thread-safe; the quit guard reads it off the UI thread.
static PROMISE_QUEUES: std::sync::Mutex<Vec<(String, Retained<NSOperationQueue>)>> = std::sync::Mutex::new(Vec::new());
// operationCount is deprecated in favour of progress, but it is the queue's own count of
// queued and running operations (AppKit adds them; nothing else of ours sees them).
#[allow(deprecated)]
pub fn promise_busy() -> usize {
    PROMISE_QUEUES.lock().unwrap().iter().map(|(_, q)| q.operationCount()).sum()
}
fn forget_queue(drag: &str) {
    PROMISE_QUEUES.lock().unwrap().retain(|(d, _)| d != drag);
}
/// Queues no promise can use any more (UI thread: the promise registry lives there), kept
/// while an operation still runs (a completion's tail).
#[allow(deprecated)]
fn prune_queues() {
    let wanted: std::collections::HashSet<String> = PROMISES.with(|p| p.borrow().values().map(|d| d.ivars().drag.clone()).collect());
    PROMISE_QUEUES.lock().unwrap().retain(|(d, q)| wanted.contains(d) || q.operationCount() > 0);
}
thread_local! { static DRAGS: RefCell<HashMap<String,Retained<Source>>> = RefCell::new(HashMap::new()); static PROMISES: RefCell<HashMap<String,Retained<Promise>>> = RefCell::new(HashMap::new()); static CLIP: Cell<(u64,isize)> = const {Cell::new((0,0))}; }
fn emit(v: Value) {
    super::super::post(super::super::Event::Page(v));
}
fn emitter() -> files::Emit {
    Arc::new(emit)
}

thread_local! {
    static TEST_PASTEBOARD: RefCell<Option<Retained<NSPasteboard>>> = const { RefCell::new(None) };
}
/// Preserve the person's clipboard during a deliberately isolated native UI test.
/// All copy/finish/paste call sites must use the same board, including changeCount.
fn clipboard_board() -> Retained<NSPasteboard> {
    if crate::store::override_base().is_some()
        && std::env::var("ITDA_NATIVE_TEST_PRIVATE_PASTEBOARD").as_deref() == Ok("1")
    {
        TEST_PASTEBOARD.with(|slot| slot.borrow_mut().get_or_insert_with(|| {
            let board = NSPasteboard::pasteboardWithUniqueName();
            crate::log!("native fixture: private pasteboard {}", board.name());
            board
        }).clone())
    } else {
        NSPasteboard::generalPasteboard()
    }
}
/// Only the test's unique board exists here; never clear the general pasteboard.
pub fn release_test_pasteboard() {
    TEST_PASTEBOARD.with(|slot| {
        if let Some(board) = slot.borrow_mut().take() {
            board.clearContents();
            // Public AppKit API, omitted from the generated bindings (oneway void).
            unsafe { let _: () = msg_send![&*board, releaseGlobally]; }
        }
    });
}

pub fn copy(v: Value) {
    let id = files::scoped_id(v["id"].as_str().unwrap_or(""), &v["window"]);
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
    let pb = clipboard_board();
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
    let pb = clipboard_board();
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
    let pb = clipboard_board();
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
        json!({"type":"files_native_paste","id":files::scoped_id(v["id"].as_str().unwrap_or(""), &v["window"]),"internal":internal,"paths":paths,"side":v["side"],"path":v["path"]}),
    );
}

struct PromiseState {
    file: File,
    token: String,
    /// The drag it belongs to (its queue's key).
    drag: String,
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
            // The provider's delegate is weak. Keep it alive even if quitting or a
            // queued completion removes its main-thread registry entry meanwhile.
            let _keep_alive = self.retain();
            files::PROMISE_WRITE.with(|p| p.set(true));
            let result = if url.isFileURL() { url.path() } else { None }
                .ok_or_else(|| std::io::Error::other("Finder 복사 위치를 확인할 수 없습니다"))
                .and_then(|path| {
                    files::export::write(
                        &self.ivars().file,
                        &PathBuf::from(path.to_string()),
                        emitter(),
                    )
                });
            files::PROMISE_WRITE.with(|p| p.set(false));
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
// Describe the private token as part of each writer before AppKit creates its
// pasteboard items. The native promise/URL representations keep their own data.
fn with_drag_type(types: &NSArray<NSString>) -> Retained<NSArray<NSString>> {
    let mut types: Vec<_> = types.iter().map(|kind| kind.retain()).collect();
    types.push(NSString::from_str("dev.jiun.itda.file-drag"));
    NSArray::from_retained_slice(&types)
}
struct ProviderState {
    token: String,
}
define_class!(
    #[unsafe(super(NSFilePromiseProvider))]
    #[thread_kind=AnyThread]
    #[name = "ItdaTokenFilePromiseProvider"]
    #[ivars=ProviderState]
    struct TokenProvider;
    unsafe impl NSObjectProtocol for TokenProvider {}
    unsafe impl NSPasteboardWriting for TokenProvider {
        #[unsafe(method(writingOptionsForType:pasteboard:))]
        fn options(&self, kind: &NSString, board: &NSPasteboard) -> NSPasteboardWritingOptions {
            if kind.to_string() == "dev.jiun.itda.file-drag" {
                NSPasteboardWritingOptions::empty()
            } else {
                unsafe { msg_send![super(self),writingOptionsForType:kind,pasteboard:board] }
            }
        }
        #[unsafe(method_id(writableTypesForPasteboard:))]
        fn types(&self, board: &NSPasteboard) -> Retained<NSArray<NSString>> {
            let native: Retained<NSArray<NSString>> =
                unsafe { msg_send![super(self),writableTypesForPasteboard:board] };
            with_drag_type(&native)
        }
        #[unsafe(method_id(pasteboardPropertyListForType:))]
        fn value(&self, kind: &NSString) -> Option<Retained<AnyObject>> {
            if kind.to_string() == "dev.jiun.itda.file-drag" {
                Some(
                    NSString::from_str(&self.ivars().token)
                        .into_super()
                        .into_super(),
                )
            } else {
                unsafe { msg_send![super(self),pasteboardPropertyListForType:kind] }
            }
        }
    }
);
struct UrlState {
    url: Retained<NSURL>,
    token: String,
}
define_class!(
    #[unsafe(super(NSObject))]
    #[thread_kind=AnyThread]
    #[name = "ItdaTokenFileUrlWriter"]
    #[ivars=UrlState]
    struct TokenUrl;
    unsafe impl NSObjectProtocol for TokenUrl {}
    unsafe impl NSPasteboardWriting for TokenUrl {
        #[unsafe(method_id(writableTypesForPasteboard:))]
        fn types(&self, board: &NSPasteboard) -> Retained<NSArray<NSString>> {
            let native = self.ivars().url.writableTypesForPasteboard(board);
            with_drag_type(&native)
        }
        #[unsafe(method_id(pasteboardPropertyListForType:))]
        fn value(&self, kind: &NSString) -> Option<Retained<AnyObject>> {
            if kind.to_string() == "dev.jiun.itda.file-drag" {
                Some(
                    NSString::from_str(&self.ivars().token)
                        .into_super()
                        .into_super(),
                )
            } else {
                self.ivars().url.pasteboardPropertyListForType(kind)
            }
        }
    }
);
fn url_writer(path: &str, token: &str) -> Retained<TokenUrl> {
    let object = TokenUrl::alloc().set_ivars(UrlState {
        url: NSURL::fileURLWithPath(&NSString::from_str(path)),
        token: token.into(),
    });
    unsafe { msg_send![super(object), init] }
}
fn promise_writer(
    file: File,
    queue: &NSOperationQueue,
    drag: &str,
) -> (Retained<TokenProvider>, String) {
    let token = crate::store::new_id();
    let kind = NSString::from_str(if file.dir {
        "public.folder"
    } else {
        "public.data"
    });
    let object = Promise::alloc().set_ivars(PromiseState {
        file,
        token: token.clone(),
        drag: drag.into(),
        queue: queue.retain(),
    });
    let delegate: Retained<Promise> = unsafe { msg_send![super(object), init] };
    let object = TokenProvider::alloc().set_ivars(ProviderState { token: drag.into() });
    let provider: Retained<TokenProvider> = unsafe {
        msg_send![super(object),initWithFileType:&*kind,delegate:ProtocolObject::<dyn NSFilePromiseProviderDelegate>::from_ref(&*delegate)]
    };
    PROMISES.with(|p| p.borrow_mut().insert(token.clone(), delegate));
    (provider, token)
}
struct PendingDrag {
    token: String,
    promises: Vec<String>,
    started: bool,
}
impl Drop for PendingDrag {
    fn drop(&mut self) {
        if !self.started {
            forget_queue(&self.token);
            files::drag::release(&self.token);
            PROMISES.with(|p| {
                let mut p = p.borrow_mut();
                for token in &self.promises {
                    p.remove(token);
                }
            });
        }
    }
}
struct DragState {
    drag: String,
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
            let _keep_alive = self.retain();
            let internal = files::drag::finish(&self.ivars().drag);
            if operation == NSDragOperation::None || internal {
                forget_queue(&self.ivars().drag);
                let _ = PROMISES.try_with(|p| {
                    let mut p = p.borrow_mut();
                    for token in &self.ivars().tokens {
                        p.remove(token);
                    }
                });
            }
            emit(
                json!({"type":"files_native_drag_end","id":self.ivars().id,"drag":self.ivars().drag,"dropped":operation!=NSDragOperation::None}),
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
    let mut pending = PendingDrag {token:files::drag::begin(&v,&selection).map_err(|e|e.to_string())?,promises:Vec::new(),started:false};
    let token = pending.token.clone();
    let queue = NSOperationQueue::new();
    queue.setMaxConcurrentOperationCount(2);
    prune_queues();
    PROMISE_QUEUES.lock().unwrap().push((token.clone(), queue.clone()));
    let mut items = Vec::new();
    for file in selection {
        let writer: Retained<ProtocolObject<dyn NSPasteboardWriting>> = if file.remote {
            let (provider,promise)=promise_writer(file,&queue,&token);
            pending.promises.push(promise);
            ProtocolObject::from_retained(provider)
        } else {
            let path = file.path().map_err(|e| e.to_string())?;
            std::fs::symlink_metadata(&path).map_err(|e| e.to_string())?;
            ProtocolObject::from_retained(url_writer(&path.to_string_lossy(),&token))
        };
        let item = NSDraggingItem::initWithPasteboardWriter(NSDraggingItem::alloc(), &writer);
        unsafe { item.setDraggingFrame_contents(frame, Some(&image)) };
        items.push(item);
    }
    let state = Source::alloc(mtm).set_ivars(DragState {
        drag: token.clone(),
        tokens:pending.promises.clone(),
        id: files::scoped_id(v["id"].as_str().unwrap_or(""), &v["window"]),
    });
    let source: Retained<Source> = unsafe { msg_send![super(state), init] };
    let timestamp = NSApp(mtm).currentEvent().map_or(0., |e| e.timestamp());
    let event=NSEvent::mouseEventWithType_location_modifierFlags_timestamp_windowNumber_context_eventNumber_clickCount_pressure(NSEventType::LeftMouseDragged,position,NSEventModifierFlags::empty(),timestamp,window.windowNumber(),None,0,1,1.).ok_or("드래그 이벤트를 만들지 못했습니다")?;
    // NSDraggingSource is usually a view; our standalone source needs an explicit
    // owner until the asynchronous end callback has returned.
    DRAGS.with(|d| {
        d.borrow_mut().insert(token, source.clone());
    });
    content.beginDraggingSessionWithItems_event_source(
        &NSArray::from_retained_slice(&items),
        &event,
        ProtocolObject::from_ref(&*source),
    );
    pending.started = true;
    Ok(())
}

pub fn register_drop(view: &NSView) {
    let mut types: Vec<_> = view.registeredDraggedTypes().iter().map(|kind|kind.retain()).collect();
    types.push(NSString::from_str("dev.jiun.itda.file-drag"));
    view.registerForDraggedTypes(&NSArray::from_retained_slice(&types));
}
fn board_origin(board: &NSPasteboard) -> Option<Value> {
    let token = board.stringForType(&NSString::from_str("dev.jiun.itda.file-drag"))?.to_string();
    files::drag::accept(&token)
}
pub fn drop_origin() -> Option<Value> {
    board_origin(&NSPasteboard::pasteboardWithName(unsafe { NSPasteboardNameDrag }))
}

pub fn promise_done(v: &Value) {
    if let Some(token) = v["token"].as_str() {
        PROMISES.with(|p| {
            p.borrow_mut().remove(token);
        });
    }
    prune_queues();
}
pub fn drag_done(v: &Value) {
    if let Some(token) = v["drag"].as_str() {
        let _ = DRAGS.try_with(|d| {
            d.borrow_mut().remove(token);
        });
    }
}
pub fn shutdown() {
    let drags = DRAGS.with(|d| std::mem::take(&mut *d.borrow_mut()));
    // AppKit has no public drag-session cancellation API. At process shutdown
    // keep any still-active source alive through late AppKit teardown callbacks.
    // Sources hold only Rust metadata, not windows; normal completed drags are
    // removed by drag_done(). The OS reclaims these on process exit.
    for source in drags.into_values() {
        std::mem::forget(source);
    }
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
    fn private_drag_board_preserves_urls_and_checks_native_session() {
        let board = NSPasteboard::pasteboardWithUniqueName();
        board.clearContents();
        let v = json!({"id":"source","window":"source-window","side":"local","path":"/tmp","items":[{"name":"itda-drag-fixture.txt"}]});
        let files = files::export::selection(&v).unwrap();
        let token = files::drag::begin(&v,&files).unwrap();
        assert!(board.writeObjects(&NSArray::from_retained_slice(&[ProtocolObject::<dyn NSPasteboardWriting>::from_retained(url_writer("/tmp/itda-drag-fixture.txt",&token))])));
        assert!(board.types().unwrap().containsObject(unsafe { NSPasteboardTypeFileURL }),"marker removed native file URLs");
        let origin = board_origin(&board).unwrap();
        assert_eq!(origin["window"],"source-window"); assert_eq!(origin["names"],json!(["itda-drag-fixture.txt"]));
        assert!(board_origin(&board).is_none(),"drop token could be replayed");
        files::drag::finish(&token); files::drag::release(&token);
        board.clearContents();
        assert!(board.writeObjects(&NSArray::from_retained_slice(&[ProtocolObject::<dyn NSPasteboardWriting>::from_retained(url_writer("/tmp/itda-drag-fixture.txt","00000000000000000000000000000000"))])));
        assert!(board_origin(&board).is_none());
        board.clearContents();
    }

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
        let (provider,promise_token) = promise_writer(file,&queue,"promise-drag-token");
        let protocol = provider.delegate().unwrap();
        let board = NSPasteboard::pasteboardWithUniqueName();
        board.clearContents();
        assert!(board.writeObjects(&NSArray::from_retained_slice(&[ProtocolObject::<dyn NSPasteboardWriting>::from_retained(provider.clone())])));
        assert_eq!(provider.writingOptionsForType_pasteboard(&NSString::from_str("dev.jiun.itda.file-drag"),&board),NSPasteboardWritingOptions::empty());
        assert_eq!(board.stringForType(&NSString::from_str("dev.jiun.itda.file-drag")).unwrap().to_string(),"promise-drag-token");
        let native_types:Retained<NSArray<NSString>>=unsafe {msg_send![super(&*provider),writableTypesForPasteboard:&*board]};
        for kind in native_types.iter() {
            assert!(board.types().unwrap().containsObject(&kind),"native promise type removed");
            let forwarded=provider.pasteboardPropertyListForType(&kind);
            let native:Option<Retained<AnyObject>>=unsafe {msg_send![super(&*provider),pasteboardPropertyListForType:&*kind]};
            match (forwarded,native) {
                (Some(a),Some(b)) => {let same:bool=unsafe {msg_send![&*a,isEqual:&*b]};assert!(same,"promise metadata changed for {kind}");},
                (None,None)=>{},_=>panic!("promise metadata missing for {kind}")
            }
        }
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
        PROMISES.with(|p|p.borrow_mut().remove(&promise_token));
        board.clearContents();
        std::fs::remove_dir_all(root).unwrap();
    }

    /// Finder's queued and running promise writes keep a quit waiting; an idle queue (a
    /// promise Finder never asked for) does not.
    #[test]
    #[allow(deprecated)]
    fn queued_promise_writes_count_until_their_operations_end() {
        let drag = format!("count-test-{}", crate::store::new_id());
        let queue = NSOperationQueue::new();
        queue.setMaxConcurrentOperationCount(1);
        PROMISE_QUEUES.lock().unwrap().push((drag.clone(), queue.clone()));
        let mine = || PROMISE_QUEUES.lock().unwrap().iter().filter(|(d, _)| *d == drag).map(|(_, q)| q.operationCount()).sum::<usize>();
        assert_eq!(mine(), 0, "an idle promise queue counts");
        let gate = Arc::new((Mutex::new(false), std::sync::Condvar::new()));
        for _ in 0..2 {
            let g = gate.clone();
            let block = block2::RcBlock::new(move || {
                let (open, cv) = &*g;
                let mut o = open.lock().unwrap();
                while !*o { o = cv.wait(o).unwrap(); }
            });
            let _: () = unsafe { msg_send![&*queue, addOperationWithBlock: &*block] };
        }
        // One running, one queued behind it: both count.
        assert_eq!(mine(), 2);
        assert!(promise_busy() >= 2);
        { let (open, cv) = &*gate; *open.lock().unwrap() = true; cv.notify_all(); }
        queue.waitUntilAllOperationsAreFinished();
        assert_eq!(mine(), 0);
        forget_queue(&drag);
        assert!(!PROMISE_QUEUES.lock().unwrap().iter().any(|(d, _)| *d == drag));
    }

    /// While Itda stops for a quit, a promise write that starts gets an error at once (Finder
    /// is told, nothing is written) instead of waiting or being dropped.
    #[test]
    fn a_promise_write_after_stopping_completes_with_an_error() {
        let root = std::env::temp_dir().join(format!("itda-promise-closed-{}", crate::store::new_id()));
        std::fs::create_dir_all(root.join("source")).unwrap();
        std::fs::write(root.join("source/a.txt"), b"a").unwrap();
        let file = File { id: "promise-closed".into(), remote: false, source: root.to_string_lossy().into_owned(), name: "source".into(), dir: true };
        let queue = NSOperationQueue::new();
        let (provider, token) = promise_writer(file, &queue, "promise-closed-drag");
        let protocol = provider.delegate().unwrap();
        let target = root.join("Finder copy");
        let url = NSURL::fileURLWithPath(&NSString::from_str(&target.to_string_lossy()));
        let errors = Arc::new(Mutex::new(Vec::new()));
        let recorded = errors.clone();
        let done = block2::RcBlock::new(move |error: *mut NSError| { recorded.lock().unwrap().push(!error.is_null()); });
        crate::files::close_admissions();
        protocol.filePromiseProvider_writePromiseToURL_completionHandler(&provider, &url, &done);
        crate::files::open_admissions();
        assert_eq!(*errors.lock().unwrap(), [true], "the promise was not completed with an error");
        assert!(!target.exists(), "a promise was written while Itda was stopping");
        PROMISES.with(|p| p.borrow_mut().remove(&token));
        std::fs::remove_dir_all(root).unwrap();
    }
}
