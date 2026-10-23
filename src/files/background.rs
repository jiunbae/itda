//! Transfers that outlive their file window. A copy or move keeps running when its window
//! closes (as in Finder or Explorer); the job is then the app's, not that page's: its events
//! go to every Itda page as `window: "app"`, the main window and any file window can show and
//! cancel it, and what became of it stays here (in memory only, a few at a time) until it is
//! dismissed. While its window is open a job stays that page's alone, as before.
use super::*;

/// Background transfers that ended and were not dismissed yet, at most.
const ENDED: usize = 30;
/// Names a retry can be offered for (the page lists them again); a bigger batch is shown
/// without the retry.
const RETRY_NAMES: usize = 1000;

struct Record {
    job: String,
    /// Counts this record's events: a page keeps the newest (a snapshot and live events may
    /// arrive out of order across threads).
    seq: u64,
    page: String,
    orphan: bool,
    ended: bool,
    /// The job's latest event, merged (progress, outcome, completed names).
    event: Value,
    /// What was asked: devices, folders, names and options. Never a token or a secret.
    request: Value,
    emit: Emit,
}

static RECORDS: OnceLock<Mutex<Vec<Record>>> = OnceLock::new();
fn records() -> &'static Mutex<Vec<Record>> {
    RECORDS.get_or_init(Default::default)
}

const TERMINAL: [&str; 3] = ["completed", "failed", "cancelled"];
fn terminal(v: &Value) -> bool {
    TERMINAL.contains(&v["state"].as_str().unwrap_or(""))
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
                all.push(Record { job: job.clone(), seq: 0, page: page_id.clone(), orphan: false, ended: false, event: json!({}), request: request.clone(), emit: raw.clone() });
            }
            let i = all.iter().position(|r| r.job == job).unwrap();
            let r = &mut all[i];
            if let (Some(to), Some(from)) = (r.event.as_object_mut(), m.as_object()) {
                for (k, x) in from { if k != "window" { to.insert(k.clone(), x.clone()); } }
            } else {
                r.event = m.clone();
            }
            r.ended = terminal(&m);
            if r.orphan {
                out = Some(shown(r));
            } else if r.ended {
                // Its page has it: nothing to keep here.
                all.remove(i);
            }
            if out.is_some() { pruned = prune(&mut all); }
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
fn prune(all: &mut Vec<Record>) -> Vec<(Emit, Value)> {
    let mut gone = Vec::new();
    while all.iter().filter(|r| r.ended).count() > ENDED {
        let i = all.iter().position(|r| r.ended).unwrap();
        let r = all.remove(i);
        gone.push((r.emit.clone(), dismissed(&r)));
    }
    gone
}
fn dismissed(r: &Record) -> Value {
    json!({"type": "files_job", "window": "app", "orphan": true, "job": r.job, "id": r.event["id"], "dismissed": true, "seq": u64::MAX})
}

/// The page's window closed: its live transfers continue as the app's. Returns how many.
pub(super) fn adopt(page: &str) -> usize {
    let mut all = records().lock().unwrap();
    let mut n = 0;
    let mut notice: Option<Emit> = None;
    let mut events = Vec::new();
    for r in all.iter_mut().filter(|r| r.page == page && !r.orphan && !r.ended) {
        r.orphan = true;
        let e = shown(r);
        n += 1;
        notice.get_or_insert_with(|| r.emit.clone());
        events.push((r.emit.clone(), e));
    }
    drop(all);
    for (emit, e) in events { emit(e); }
    if let Some(emit) = notice {
        emit(json!({"type": "files_background", "window": "app", "count": n}));
    }
    n
}

/// Whether `job` is an adopted (app) transfer, which any Itda page may cancel or dismiss.
pub(super) fn is_orphan(job: &str) -> bool {
    records().lock().unwrap().iter().any(|r| r.job == job && r.orphan)
}

/// Forget an ended background transfer (the person dismissed it).
pub(super) fn dismiss(job: &str) {
    let mut all = records().lock().unwrap();
    if let Some(i) = all.iter().position(|r| r.job == job && r.orphan && r.ended) {
        let r = all.remove(i);
        drop(all);
        (r.emit)(dismissed(&r));
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
        let t = track(&request, json!(page), Arc::new(|_| {}));
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
    }

    #[test]
    fn a_big_batch_is_shown_without_its_names() {
        let names: Vec<String> = (0..RETRY_NAMES + 1).map(|i| i.to_string()).collect();
        let s = summary(&json!({"names": names}));
        assert!(s["names"].is_null() && s["count"] == RETRY_NAMES + 1);
    }
}
