//! Transfers that outlive their file window. A copy or move keeps running when its window
//! closes (as in Finder or Explorer); the job is then the app's, not that page's: its events
//! go to every Itda page as `window: "app"`, the main window and any file window can show and
//! cancel it, and a bounded history of failed or cancelled transfers survives app restarts.
//! While its window is open a job stays that page's alone, as before. If the window
//! closes, its retained outcomes are also available in new windows' snapshots.
use super::*;
use std::io::{Read, Write};

/// Transfers that ended and were not dismissed yet, at most.
const ENDED: usize = 30;
/// Names a retry can be offered for (the page lists them again); a bigger batch is shown
/// without the retry.
const RETRY_NAMES: usize = 1000;
const HISTORY_FILE: &str = "file-transfer-history-v1.json";
const HISTORY_RECORD_BYTES: usize = 64 * 1024;
const HISTORY_BYTES: usize = ENDED * (HISTORY_RECORD_BYTES + 1) + 1;

struct Record {
    job: String,
    /// Counts this record's events: a page keeps the newest (a snapshot and live events may
    /// arrive out of order across threads).
    seq: u64,
    page: String,
    orphan: bool,
    ended: bool,
    /// When it ended (a counter): the history keeps the latest outcomes, whatever the start order.
    ended_at: u64,
    /// The job's latest event, merged (progress, outcome, completed names).
    event: Value,
    /// What was asked: devices, folders, names and options. Never a token or a secret.
    request: Value,
    /// Restored outcomes have no originating page or emitter.
    emit: Option<Emit>,
}

static RECORDS: OnceLock<Mutex<Vec<Record>>> = OnceLock::new();
static ENDINGS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
/// Pages whose window closed (most recent last): a transfer they asked for whose first event
/// comes only after that is the app's from the start.
static RELEASED: Mutex<std::collections::VecDeque<String>> = Mutex::new(std::collections::VecDeque::new());
const RELEASED_KEPT: usize = 64;
fn records() -> &'static Mutex<Vec<Record>> {
    RECORDS.get_or_init(|| {
        let all = history_dir().map_or_else(Vec::new, |dir| load_history(&dir));
        ENDINGS.store(all.len() as u64, Ordering::Relaxed);
        Mutex::new(all)
    })
}

const TERMINAL: [&str; 3] = ["completed", "failed", "cancelled"];
fn terminal(v: &Value) -> bool {
    TERMINAL.contains(&v["state"].as_str().unwrap_or(""))
}

fn retryable_outcome(v: &Value) -> bool {
    matches!(v["state"].as_str(), Some("failed" | "cancelled"))
}

fn history_dir() -> Option<PathBuf> {
    // Unit tests exercising the process-global records must never touch the user's data.
    #[cfg(test)]
    return None;
    #[cfg(not(test))]
    Some(crate::store::dir())
}

/// Rebuild the history entry from known fields. In particular, errors and progress details
/// can include backend diagnostics, while req/window/clipboard/drag identify live requests.
fn history_entry(event: &Value, request: &Value) -> Option<Value> {
    if !retryable_outcome(event) { return None; }
    let job = event["job"].as_str().filter(|s| !s.is_empty())?;
    let id = event["id"].as_str()?;
    let count = request["count"].as_u64().unwrap_or(0);
    fn strings(a: &Value) -> Option<&Vec<Value>> {
        a.as_array().filter(|items| items.len() <= RETRY_NAMES && items.iter().all(Value::is_string))
    }
    let names = strings(&request["names"]).filter(|items| items.len() as u64 == count && !items.is_empty());
    let completed = strings(&event["completed_names"]);
    let partial = event["partial"]["name"].as_str().zip(event["partial"]["dir"].as_bool());
    // Keep the complete set of finished names whenever retry is offered. Losing even one
    // could cause a confirmed retry to copy or move an item that already finished.
    let can_retry = names.is_some() && completed.is_some()
        && (event["partial"].is_null() || partial.is_some())
        && ["local", "remote"].contains(&request["from"].as_str().unwrap_or(""))
        && ["local", "remote"].contains(&request["to"].as_str().unwrap_or(""))
        && request["source"].is_string() && request["destination"].is_string();
    let mut entry = json!({
        "type": "files_job", "job": job, "id": id, "state": event["state"],
        "moving": request["move"] == true,
        "engine": event["engine"].as_str().filter(|s| ["sftp", "rsync"].contains(s)),
        "completed_names": completed.filter(|_| can_retry),
        "partial": partial.filter(|_| can_retry).map(|(name, dir)| json!({"name": name, "dir": dir})),
        "source_closed": event["source_closed"] == true,
        "request": {
            "id": id, "from": request["from"].as_str().filter(|s| ["local", "remote"].contains(s)),
            "to": request["to"].as_str().filter(|s| ["local", "remote"].contains(s)),
            "source": request["source"].as_str(), "destination": request["destination"].as_str(),
            "names": names.filter(|_| can_retry), "count": count, "move": request["move"] == true,
            "policy": request["policy"].as_str().filter(|s| ["skip", "keep", "replace"].contains(s)),
            "engine": request["engine"].as_str().filter(|s| ["auto", "sftp", "rsync"].contains(s)),
            "from_id": request["from_id"].as_str().filter(|s| !s.is_empty()),
        },
    });
    if serde_json::to_vec(&entry).ok()?.len() <= HISTORY_RECORD_BYTES {
        return Some(entry);
    }
    entry["request"]["names"] = Value::Null;
    entry["completed_names"] = Value::Null;
    entry["partial"] = Value::Null;
    (serde_json::to_vec(&entry).ok()?.len() <= HISTORY_RECORD_BYTES).then_some(entry)
}

fn load_history(dir: &Path) -> Vec<Record> {
    let read = || -> io::Result<Vec<Value>> {
        let mut data = Vec::new();
        std::fs::File::open(dir.join(HISTORY_FILE))?
            .take((HISTORY_BYTES + 1) as u64)
            .read_to_end(&mut data)?;
        if data.len() > HISTORY_BYTES {
            return Err(io::Error::new(io::ErrorKind::InvalidData, "transfer history is oversized"));
        }
        serde_json::from_slice(&data).map_err(io::Error::other)
    };
    let entries = match read() {
        Ok(entries) => entries,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Vec::new(),
        Err(e) => {
            crate::log!("file transfer history could not be read ({:?})", e.kind());
            return Vec::new();
        }
    };
    let mut all: Vec<Record> = Vec::new();
    for entry in entries.iter().rev() {
        let Some(mut event) = history_entry(entry, &entry["request"]) else { continue };
        let job = get(&event, "job");
        if all.iter().any(|r| r.job == job) { continue; }
        let request = event.as_object_mut().unwrap().remove("request").unwrap();
        all.push(Record { job, seq: 0, page: String::new(), orphan: true, ended: true, ended_at: 0, event, request, emit: None });
        if all.len() == ENDED { break; }
    }
    all.reverse();
    for (i, r) in all.iter_mut().enumerate() { r.ended_at = i as u64 + 1; }
    all
}

fn save_history(dir: &Path, all: &[Record]) -> io::Result<()> {
    let mut ended: Vec<_> = all.iter().filter(|r| r.ended).collect();
    ended.sort_unstable_by_key(|r| r.ended_at);
    let entries: Vec<_> = ended.into_iter().filter_map(|r| history_entry(&r.event, &r.request)).collect();
    let data = serde_json::to_vec(&entries).map_err(io::Error::other)?;
    // The home process is the sole writer and holds the records lock across replacement.
    let tmp = dir.join(format!("{HISTORY_FILE}.tmp"));
    let result = (|| {
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create(true).truncate(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options.open(&tmp)?;
        file.write_all(&data)?;
        file.sync_all()?;
        drop(file);
        std::fs::rename(&tmp, dir.join(HISTORY_FILE))
    })();
    if result.is_err() { let _ = std::fs::remove_file(&tmp); }
    result
}

fn persist(all: &[Record]) {
    if let Some(dir) = history_dir() {
        if let Err(e) = save_history(&dir, all) {
            crate::log!("file transfer history could not be saved ({:?})", e.kind());
        }
    }
}

/// The non-secret summary of a transfer request (no from_window, clipboard or drag token).
fn summary(v: &Value) -> Value {
    let names = v["names"].as_array().filter(|n| n.len() <= RETRY_NAMES).cloned();
    json!({
        "id": v["id"], "from": v["from"], "to": v["to"], "source": v["source"], "destination": v["destination"],
        "names": names, "count": v["names"].as_array().map_or(0, |n| n.len()),
        "move": v["move"] == true, "policy": v["policy"], "engine": v["engine"],
        "from_id": v["from_id"].as_str().filter(|f| !f.is_empty()),
    })
}

fn shown(r: &mut Record) -> Value {
    r.seq += 1;
    let mut e = r.event.clone();
    e["seq"] = json!(r.seq);
    e["window"] = json!("app");
    e["orphan"] = json!(true);
    e["request"] = r.request.clone();
    e
}

/// Wraps a transfer's events: the page that started it gets them as before; once that page
/// is gone they are the app's. `page` is the requesting page's window token.
pub(super) fn track(v: &Value, page: Value, emit: Emit) -> Emit {
    let request = summary(v);
    let page_id = page.as_str().unwrap_or("").to_string();
    let raw = emit.clone();
    Arc::new(move |mut m: Value| {
        let job = m["job"].as_str().unwrap_or("").to_string();
        let mut out = None;
        let mut pruned = Vec::new();
        if !job.is_empty() && m["type"] == "files_job" {
            let mut all = records().lock().unwrap();
            if !all.iter().any(|r| r.job == job) {
                let orphan = RELEASED.lock().unwrap().contains(&page_id);
                all.push(Record { job: job.clone(), seq: 0, page: page_id.clone(), orphan, ended: false, ended_at: 0, event: json!({}), request: request.clone(), emit: Some(raw.clone()) });
            }
            let i = all.iter().position(|r| r.job == job).unwrap();
            let r = &mut all[i];
            if let (Some(to), Some(from)) = (r.event.as_object_mut(), m.as_object()) {
                for (k, x) in from { if k != "window" { to.insert(k.clone(), x.clone()); } }
            } else {
                r.event = m.clone();
            }
            if terminal(&m) && !r.ended { r.ended_at = ENDINGS.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1; }
            r.ended = terminal(&m);
            let mut history_changed = retryable_outcome(&m);
            if r.orphan {
                out = Some(shown(r));
            } else if history_changed {
                m["persistent"] = json!(true);
            } else if r.ended {
                // Successful transfers owned by an open page are visible on that page.
                all.remove(i);
            }
            if out.is_some() || history_changed {
                for r in prune(&mut all) {
                    history_changed |= retryable_outcome(&r.event);
                    pruned.push((r.emit.clone().unwrap_or_else(|| raw.clone()), dismissed(&r)));
                }
            }
            if history_changed { persist(&all); }
        }
        for (emit, e) in pruned { emit(e); }
        match out {
            Some(e) => raw(e),
            None => { m["window"] = page.clone(); raw(m) }
        }
    })
}

/// Keeps at most ENDED ended records (the oldest go first); live ones always stay. Pages are
/// told what was dropped, so their lists match what a page opening now would get.
fn prune(all: &mut Vec<Record>) -> Vec<Record> {
    let mut gone = Vec::new();
    while all.iter().filter(|r| r.ended).count() > ENDED {
        let i = all.iter().enumerate().filter(|(_, r)| r.ended).min_by_key(|(_, r)| r.ended_at).map(|(i, _)| i).unwrap();
        gone.push(all.remove(i));
    }
    gone
}
fn dismissed(r: &Record) -> Value {
    json!({"type": "files_job", "window": "app", "orphan": true, "job": r.job, "id": r.event["id"], "dismissed": true, "seq": u64::MAX})
}

/// The page's window closed: its live transfers continue as the app's. Returns how many.
pub(super) fn adopt(page: &str) -> usize {
    {
        let mut released = RELEASED.lock().unwrap();
        if !released.iter().any(|p| p == page) { released.push_back(page.to_string()); }
        while released.len() > RELEASED_KEPT { released.pop_front(); }
    }
    let mut all = records().lock().unwrap();
    let mut n = 0;
    let mut notice: Option<Emit> = None;
    let mut events = Vec::new();
    for r in all.iter_mut().filter(|r| r.page == page && !r.orphan) {
        r.orphan = true;
        let mut e = shown(r);
        if let Some(emit) = &r.emit {
            if r.ended {
                // The original page already announced this outcome.
                e["quiet"] = json!(true);
            } else {
                n += 1;
                notice.get_or_insert_with(|| emit.clone());
            }
            events.push((emit.clone(), e));
        }
    }
    drop(all);
    for (emit, e) in events { emit(e); }
    if let Some(emit) = notice {
        emit(json!({"type": "files_background", "window": "app", "count": n}));
    }
    n
}

/// The page is in use again (the main window's file browser reopens with the same token):
/// what it starts from now on is its own.
pub(super) fn in_use(page: &str) {
    let mut released = RELEASED.lock().unwrap();
    if !released.is_empty() { released.retain(|p| p != page); }
}

/// Whether `job` is an adopted (app) transfer, which any Itda page may cancel or dismiss.
pub(super) fn is_orphan(job: &str) -> bool {
    records().lock().unwrap().iter().any(|r| r.job == job && r.orphan)
}

/// Forget an ended transfer from its owning page, or from any page once adopted.
pub(super) fn dismiss(job: &str, page: &str, emit: Emit) {
    let mut all = records().lock().unwrap();
    if let Some(i) = all.iter().position(|r| r.job == job && r.ended && (r.orphan || r.page == page)) {
        let r = all.remove(i);
        if retryable_outcome(&r.event) { persist(&all); }
        drop(all);
        (r.emit.as_ref().unwrap_or(&emit))(dismissed(&r));
    }
}

/// The app's transfers for a page that opens (or reloads): what is running and what ended.
pub fn snapshot() -> Value {
    Value::Array(records().lock().unwrap().iter_mut().filter(|r| r.orphan).map(shown).collect())
}


#[cfg(test)]
mod tests {
    use super::*;

    fn collector() -> (Emit, Arc<Mutex<Vec<Value>>>) {
        let seen = Arc::new(Mutex::new(Vec::new()));
        let s = seen.clone();
        (Arc::new(move |e: Value| s.lock().unwrap().push(e)), seen)
    }
    fn mine(job: &str) -> Option<Value> {
        snapshot().as_array().unwrap().iter().find(|e| e["job"] == job).cloned()
    }

    /// One test, in order: the records are shared by the whole process.
    #[test]
    fn a_closed_window_hands_its_transfers_to_the_app() {
        let host = Host { id: crate::store::new_id(), ..Default::default() };
        let (page, other) = (crate::store::new_id(), crate::store::new_id());
        let (emit, seen) = collector();
        let request = json!({"op":"transfer","id":host.id,"from":"remote","to":"remote","source":"/a","destination":"/b",
            "names":["x","y"],"move":true,"policy":"keep","engine":"auto","from_id":"src","from_window":"secret-page","from_clip":"clip-token","from_drag":"drag-token"});
        let job = crate::store::new_id();
        let control = Arc::new(Control::default());
        jobs().lock().unwrap().insert(job.clone(), (scoped_id(&host.id, &json!(page)), control.clone()));
        let tracked = track(&request, json!(page), emit);
        tracked(json!({"type":"files_job","id":host.id,"job":job,"state":"queued","moving":true}));
        // Open page: its own events, its own cancel; another page can't take it.
        assert_eq!(seen.lock().unwrap().last().unwrap()["window"], json!(page));
        assert!(!is_orphan(&job) && mine(&job).is_none());
        dispatch(json!({"op":"cancel_background","job":job,"window":other}), host.clone(), Login::default(), Arc::new(|_| {}));
        assert!(control.check().is_ok(), "a live page's transfer was cancelled from another page");

        // The window closes: the transfer runs on as the app's, and the app is told.
        release_page(&page);
        assert!(control.check().is_ok(), "closing the window stopped the transfer");
        {
            let s = seen.lock().unwrap();
            assert!(s.iter().any(|e| e["type"] == "files_background" && e["count"] == 1 && e["window"] == "app"), "{s:?}");
            let adopted = s.iter().rev().find(|e| e["job"] == json!(job)).unwrap();
            assert!(adopted["window"] == "app" && adopted["orphan"] == true && adopted["state"] == "queued");
        }
        tracked(json!({"type":"files_job","id":host.id,"job":job,"state":"running","file":"x","bytes":5,"total":10}));
        let running = seen.lock().unwrap().last().unwrap().clone();
        assert_eq!((running["window"].as_str(), running["bytes"].as_u64()), (Some("app"), Some(5)));
        // Each event is newer than the adoption's (a page keeps the newest).
        let adopted_seq = seen.lock().unwrap().iter().find(|e| e["job"] == json!(job) && e["window"] == "app").unwrap()["seq"].as_u64().unwrap();
        assert!(running["seq"].as_u64().unwrap() > adopted_seq);
        // A page that opens now sees it, with what was asked and nothing secret.
        let snap = mine(&job).expect("not in the snapshot");
        assert!(snap["state"] == "running" && snap["moving"] == true && snap["request"]["names"] == json!(["x","y"]) && snap["request"]["from_id"] == "src");
        let text = snap.to_string();
        assert!(!text.contains("secret-page") && !text.contains("clip-token") && !text.contains("drag-token"), "{text}");
        // Any page may now stop it.
        dispatch(json!({"op":"cancel_background","job":job,"window":other}), host.clone(), Login::default(), Arc::new(|_| {}));
        assert!(control.check().is_err(), "the app could not cancel an adopted transfer");
        jobs().lock().unwrap().remove(&job);
        tracked(json!({"type":"files_job","id":host.id,"job":job,"state":"cancelled","error":"전송을 취소했습니다","completed_names":["x"]}));
        let ended = mine(&job).expect("the outcome was dropped");
        assert!(ended["state"] == "cancelled" && ended["completed_names"] == json!(["x"]));
        // Dismissed: gone, and pages are told.
        dispatch(json!({"op":"dismiss_background","job":job,"window":other}), host.clone(), Login::default(), Arc::new(|_| {}));
        assert!(mine(&job).is_none());
        assert_eq!(seen.lock().unwrap().last().unwrap()["dismissed"], true);

        // A transfer that ends while its page is open leaves nothing behind.
        let quick = crate::store::new_id();
        let t = track(&request, json!(crate::store::new_id()), Arc::new(|_| {}));
        t(json!({"type":"files_job","id":host.id,"job":quick,"state":"queued"}));
        t(json!({"type":"files_job","id":host.id,"job":quick,"state":"completed"}));
        assert!(!records().lock().unwrap().iter().any(|r| r.job == quick));

        // Ended background transfers are bounded; running ones are always kept.
        let crowd = crate::store::new_id();
        let live = crate::store::new_id();
        let (emit, told) = collector();
        let t = track(&request, json!(crowd), emit);
        t(json!({"type":"files_job","id":host.id,"job":live,"state":"running"}));
        let ids: Vec<String> = (0..ENDED + 5).map(|_| crate::store::new_id()).collect();
        for j in &ids { t(json!({"type":"files_job","id":host.id,"job":j,"state":"queued"})); }
        adopt(&crowd);
        for j in &ids { t(json!({"type":"files_job","id":host.id,"job":j,"state":"failed","error":"x"})); }
        let all = records().lock().unwrap();
        assert!(all.iter().filter(|r| r.ended).count() <= ENDED);
        assert!(told.lock().unwrap().iter().any(|e| e["dismissed"] == true && e["job"] == json!(ids[0])), "pages were not told about a dropped outcome");
        assert!(all.iter().any(|r| r.job == live && !r.ended), "a running transfer was dropped");
        assert!(!all.iter().any(|r| r.job == ids[0]), "the oldest outcome was kept over newer ones");
        drop(all);
        // The long-running one ends last: its outcome is the newest and stays.
        t(json!({"type":"files_job","id":host.id,"job":live,"state":"completed"}));
        let all = records().lock().unwrap();
        assert!(all.iter().any(|r| r.job == live && r.ended), "the latest outcome was dropped for an older one");
        assert!(!all.iter().any(|r| r.job == ids[5]), "an older outcome was kept over the latest");
        drop(all);

        // Failures in an open page stay private while it is open, but it can dismiss them.
        let page = crate::store::new_id();
        let (emit, seen) = collector();
        let open = track(&request, json!(page), emit.clone());
        let failure = crate::store::new_id();
        open(json!({"type":"files_job","id":host.id,"job":failure,"state":"failed","completed_names":[]}));
        assert!(mine(&failure).is_none());
        {
            let events = seen.lock().unwrap();
            let ended = events.last().unwrap();
            assert_eq!(ended["window"], json!(page));
            assert_eq!(ended["persistent"], true);
        }
        dismiss(&failure, &other, emit.clone());
        assert!(records().lock().unwrap().iter().any(|r| r.job == failure));
        dismiss(&failure, &page, emit.clone());
        assert!(!records().lock().unwrap().iter().any(|r| r.job == failure));
        assert_eq!(seen.lock().unwrap().last().unwrap()["dismissed"], true);

        // Closing the page publishes the retained outcome without announcing it a second time.
        let cancelled = crate::store::new_id();
        open(json!({"type":"files_job","id":host.id,"job":cancelled,"state":"cancelled","completed_names":[]}));
        let events = seen.lock().unwrap().len();
        assert_eq!(adopt(&page), 0);
        {
            let seen = seen.lock().unwrap();
            assert_eq!(seen.len(), events + 1);
            assert_eq!(seen.last().unwrap()["window"], "app");
            assert_eq!(seen.last().unwrap()["quiet"], true);
        }
        assert_eq!(mine(&cancelled).unwrap()["state"], "cancelled");
        dismiss(&cancelled, &other, emit);
        assert!(mine(&cancelled).is_none());
    }

    /// A window that closes before its transfer's first event still hands it over; the page
    /// in use again (same token) starts its own transfers.
    #[test]
    fn a_transfer_born_after_its_window_closed_is_the_apps() {
        let page = crate::store::new_id();
        let (emit, seen) = collector();
        adopt(&page);
        let job = crate::store::new_id();
        let t = track(&json!({"names":["a"]}), json!(page), emit.clone());
        t(json!({"type":"files_job","id":"h","job":job,"state":"queued"}));
        assert_eq!(seen.lock().unwrap().last().unwrap()["window"], "app");
        assert!(is_orphan(&job));
        in_use(&page);
        let mine = crate::store::new_id();
        let t = track(&json!({"names":["a"]}), json!(page), emit);
        t(json!({"type":"files_job","id":"h","job":mine,"state":"queued"}));
        assert_eq!(seen.lock().unwrap().last().unwrap()["window"], json!(page));
        assert!(!is_orphan(&mine));
    }

    #[test]
    fn a_big_batch_is_shown_without_its_names() {
        let names: Vec<String> = (0..RETRY_NAMES + 1).map(|i| i.to_string()).collect();
        let s = summary(&json!({"names": names}));
        assert!(s["names"].is_null() && s["count"] == RETRY_NAMES + 1);
    }

    #[test]
    fn history_restores_only_safe_terminal_outcomes() -> io::Result<()> {
        let dir = std::env::temp_dir().join(format!("itda-transfer-history-{}", crate::store::new_id()));
        std::fs::create_dir(&dir)?;
        let result = (|| {
            let request = summary(&json!({
                "from":"remote","to":"local","source":"/in","destination":"/out",
                "from_id":"source-device","move":true,"policy":"keep","engine":"sftp",
                "names":["done","partial"],"from_window":"private-window","from_clip":"private-clip","from_drag":"private-drag",
            }));
            let record = |job: &str, state: &str, ended_at: u64, request: Value| Record {
                job: job.into(), seq: 12, page: "private-window".into(), orphan: true,
                ended: state != "running", ended_at, emit: None, request,
                event: json!({"job":job,"id":"device","state":state,"completed_names":["done"],
                    "partial":{"name":"partial","dir":true,"unknown":"private-partial"},
                    "req":"private-req","error":"private-error","detail":"private-detail","source_closed":true}),
            };
            let mut all = vec![
                record("newest", "cancelled", 4, request.clone()),
                record("older", "failed", 2, request.clone()),
                record("active", "running", 0, request.clone()),
                record("success", "completed", 3, request.clone()),
            ];
            let mut wide_request = request.clone();
            wide_request["names"][1] = json!("x".repeat(HISTORY_RECORD_BYTES));
            all.push(record("wide", "failed", 5, wide_request));
            save_history(&dir, &all)?;
            let data = std::fs::read_to_string(dir.join(HISTORY_FILE))?;
            assert!(!data.contains("private-") && !data.contains("\"active\"") && !data.contains("\"success\""));
            let mut restored = load_history(&dir);
            assert_eq!(restored.iter().map(|r| r.job.as_str()).collect::<Vec<_>>(), ["older", "newest", "wide"]);
            assert!(restored.iter().all(|r| r.ended && r.orphan && r.emit.is_none()));
            let event = shown(&mut restored[1]);
            assert_eq!(event["window"], "app");
            assert_eq!(event["completed_names"], json!(["done"]));
            assert_eq!(event["partial"], json!({"name":"partial","dir":true}));
            assert_eq!(event["request"]["names"], json!(["done","partial"]));
            assert_eq!(event["request"]["from_id"], "source-device");
            assert_eq!(event["moving"], true);
            assert_eq!(event["source_closed"], true);
            assert!(restored[2].request["names"].is_null());
            assert_eq!(restored[2].request["count"], 2);

            // The same replacement is used after dismissing one or all restored entries.
            restored.remove(1);
            save_history(&dir, &restored)?;
            assert_eq!(load_history(&dir).len(), 2);
            save_history(&dir, &[])?;
            assert!(load_history(&dir).is_empty());
            std::fs::write(dir.join(HISTORY_FILE), b"interrupted fixture")?;
            assert!(load_history(&dir).is_empty());
            Ok(())
        })();
        let cleanup = std::fs::remove_dir_all(&dir);
        result.and(cleanup)
    }
}
