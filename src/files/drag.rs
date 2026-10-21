//! Provenance for a native drag between this process's file windows. The OS carries
//! only a random token; source paths and credentials remain in the live session table.
use super::*;
use std::time::{Duration, Instant};
const TTL: Duration = Duration::from_secs(10 * 60);
const LIMIT: usize = 32;
struct Lease {
    origin: Value,
    key: String,
    source: Option<Arc<Connection>>,
    active: bool,
    accepted: bool,
    at: Instant,
}
fn leases() -> &'static Mutex<HashMap<String, Lease>> {
    static TABLE: OnceLock<Mutex<HashMap<String, Lease>>> = OnceLock::new();
    TABLE.get_or_init(Default::default)
}
fn prune(table: &mut HashMap<String, Lease>) {
    table.retain(|_, l| l.at.elapsed() < TTL);
}
pub fn begin(v: &Value, files: &[export::File]) -> io::Result<String> {
    let key = scoped_id(v["id"].as_str().unwrap_or(""), &v["window"]);
    let source = if v["side"] == "remote" {
        Some(connection(&key)?)
    } else {
        None
    };
    let mut random = [0; 16];
    getrandom::getrandom(&mut random).map_err(io::Error::other)?;
    let token: String = random.iter().map(|b| format!("{b:02x}")).collect();
    let origin = json!({"id":v["id"],"window":v["window"],"side":v["side"],"path":v["path"],"names":files.iter().map(|f|f.name.clone()).collect::<Vec<_>>(),"drag":token});
    let mut table = leases().lock().unwrap();
    prune(&mut table);
    if table.len() >= LIMIT {
        return Err(io::Error::other(
            "전송 확인 창을 닫은 뒤 다시 끌어 놓아 주세요",
        ));
    }
    table.insert(
        token.clone(),
        Lease {
            origin,
            key,
            source,
            active: true,
            accepted: false,
            at: Instant::now(),
        },
    );
    Ok(token)
}
pub fn valid(token: &str) -> bool {
    let mut table = leases().lock().unwrap();
    prune(&mut table);
    table.get(token).is_some_and(|l| l.active && !l.accepted)
}
pub fn accept(token: &str) -> Option<Value> {
    let mut table = leases().lock().unwrap();
    prune(&mut table);
    let lease = table.get_mut(token).filter(|l| l.active && !l.accepted)?;
    lease.accepted = true;
    Some(lease.origin.clone())
}
pub fn finish(token: &str) -> bool {
    let mut table = leases().lock().unwrap();
    let accepted = table.get_mut(token).is_some_and(|lease| {
        lease.active = false;
        lease.accepted
    });
    if !accepted {
        table.remove(token);
    }
    accepted
}
pub fn release(token: &str) {
    leases().lock().unwrap().remove(token);
}
pub(super) fn source(key: &str, token: &str) -> Option<Arc<Connection>> {
    let mut table = leases().lock().unwrap();
    prune(&mut table);
    table
        .get(token)
        .filter(|l| l.accepted && l.key == key)
        .and_then(|l| l.source.clone())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn local() -> (String, Value) {
        let v = json!({"id":"source","window":"source-window","side":"local","path":"/fixture","items":[{"name":"한글.txt"}]});
        let files = export::selection(&v).unwrap();
        (begin(&v, &files).unwrap(), v)
    }
    #[test]
    fn native_tokens_reject_stale_foreign_and_repeated_drops() {
        let (cancelled, _) = local();
        assert!(valid(&cancelled));
        assert!(!valid("forged"));
        assert!(!finish(&cancelled));
        assert!(accept(&cancelled).is_none());
        let (accepted, v) = local();
        let origin = accept(&accepted).unwrap();
        assert_eq!(origin["path"], v["path"]);
        assert_eq!(origin["window"], v["window"]);
        assert_eq!(origin["names"], json!(["한글.txt"]));
        assert!(accept(&accepted).is_none());
        assert!(finish(&accepted));
        assert!(
            leases().lock().unwrap().contains_key(&accepted),
            "accepted drop lost its confirmation lease"
        );
        release(&accepted);
        assert!(!leases().lock().unwrap().contains_key(&accepted));
        let (expired, _) = local();
        leases().lock().unwrap().get_mut(&expired).unwrap().at = Instant::now() - TTL;
        assert!(!valid(&expired));
        assert!(accept(&expired).is_none());
    }
}
