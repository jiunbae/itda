//! Bounded prepared-preview cache. A disconnected connection's in-flight work cannot refill it.
use std::{
    collections::HashMap,
    sync::{Arc, Mutex, OnceLock},
    time::{Duration, Instant},
};
const BUDGET: usize = 32 * 1024 * 1024;
const COUNT: usize = 32;
const TTL: Duration = Duration::from_secs(300);
#[derive(Clone, Debug, Hash, PartialEq, Eq)]
pub struct Key {
    connection: String,
    epoch: u64,
    path: String,
    size: u64,
    modified: u128,
}
#[derive(Clone)]
pub struct Prepared {
    pub bytes: Arc<Vec<u8>>,
    pub mime: String,
    pub first_page: bool,
}
struct Item {
    value: Prepared,
    touched: Instant,
}
#[derive(Default)]
struct Cache {
    epochs: HashMap<String, u64>,
    next: u64,
    entries: HashMap<Key, Item>,
}
impl Cache {
    fn key(&mut self, connection: &str, path: String, size: u64, modified: u128) -> Key {
        if self.epochs.len() >= 256 && !self.epochs.contains_key(connection) {
            self.epochs.clear();
            self.entries.clear();
        }
        self.next += 1;
        let epoch = *self.epochs.entry(connection.into()).or_insert(self.next);
        Key {
            connection: connection.into(),
            epoch,
            path,
            size,
            modified,
        }
    }
    fn get(&mut self, key: &Key) -> Option<Prepared> {
        if self.epochs.get(&key.connection) != Some(&key.epoch) {
            return None;
        }
        self.entries.retain(|k, v| {
            v.touched.elapsed() < TTL
                && !(k.connection == key.connection && k.path == key.path && k != key)
        });
        let v = self.entries.get_mut(key)?;
        v.touched = Instant::now();
        Some(v.value.clone())
    }
    fn insert(&mut self, key: Key, value: Prepared) {
        let size = value.bytes.len();
        if self.epochs.get(&key.connection) != Some(&key.epoch) || size > BUDGET {
            return;
        }
        self.entries.retain(|k, v| {
            v.touched.elapsed() < TTL && !(k.connection == key.connection && k.path == key.path)
        });
        while self.entries.len() >= COUNT
            || self
                .entries
                .values()
                .map(|v| v.value.bytes.len())
                .sum::<usize>()
                + size
                > BUDGET
        {
            let Some(oldest) = self
                .entries
                .iter()
                .min_by_key(|(_, v)| v.touched)
                .map(|(k, _)| k.clone())
            else {
                break;
            };
            self.entries.remove(&oldest);
        }
        self.entries.insert(
            key,
            Item {
                value,
                touched: Instant::now(),
            },
        );
    }
    fn invalidate(&mut self, connection: &str) {
        let matches = |k: &str| {
            k == connection
                || (!connection.contains('\x1f') && k.starts_with(&format!("{connection}\x1f")))
        };
        self.entries.retain(|k, _| !matches(&k.connection));
        self.epochs.retain(|k, _| !matches(k));
    }
}
fn cache() -> &'static Mutex<Cache> {
    static CACHE: OnceLock<Mutex<Cache>> = OnceLock::new();
    CACHE.get_or_init(Default::default)
}
pub fn key(connection: &str, path: String, size: u64, modified: u128) -> Key {
    cache()
        .lock()
        .unwrap()
        .key(connection, path, size, modified)
}
pub fn get(key: &Key) -> Option<Prepared> {
    cache().lock().unwrap().get(key)
}
pub fn insert(key: Key, value: Prepared) {
    cache().lock().unwrap().insert(key, value);
}
pub fn invalidate(connection: &str) {
    cache().lock().unwrap().invalidate(connection);
}

#[cfg(test)]
mod tests {
    use super::*;
    fn content(n: usize) -> Prepared {
        Prepared {
            bytes: Arc::new(vec![0; n]),
            mime: "image/png".into(),
            first_page: false,
        }
    }
    #[test]
    fn metadata_and_connection_identity_control_hits() {
        let mut c = Cache::default();
        let a = c.key("host\x1fpage", "/canonical/file".into(), 10, 20);
        c.insert(a.clone(), content(10));
        assert!(c.get(&a).is_some());
        let other = c.key("other\x1fpage", "/canonical/file".into(), 10, 20);
        assert!(c.get(&other).is_none());
        let changed = c.key("host\x1fpage", "/canonical/file".into(), 10, 21);
        assert!(c.get(&changed).is_none());
        assert!(c.get(&a).is_none());
    }
    #[test]
    fn disconnect_prevents_late_repopulation_even_after_reconnect() {
        let mut c = Cache::default();
        let old = c.key("host\x1fpage", "/file".into(), 10, 20);
        c.insert(old.clone(), content(10));
        c.invalidate("host");
        let new = c.key("host\x1fpage", "/file".into(), 10, 20);
        c.insert(old.clone(), content(10));
        assert!(c.get(&old).is_none());
        c.insert(new.clone(), content(10));
        assert!(c.get(&new).is_some());
        assert_ne!(old, new);
    }
    #[test]
    fn lru_entry_and_byte_caps_and_expiry() {
        let mut c = Cache::default();
        let a = c.key("c", "a".into(), 1, 1);
        let b = c.key("c", "b".into(), 1, 1);
        c.insert(a.clone(), content(BUDGET / 2));
        c.insert(b.clone(), content(BUDGET / 2));
        c.get(&a);
        let d = c.key("c", "d".into(), 1, 1);
        c.insert(d.clone(), content(1));
        assert!(c.get(&b).is_none());
        assert!(c.get(&a).is_some());
        c.entries.get_mut(&d).unwrap().touched = Instant::now() - TTL;
        assert!(c.get(&d).is_none());
        for i in 0..COUNT + 2 {
            let k = c.key("c", i.to_string(), 1, 1);
            c.insert(k, content(1));
        }
        assert!(c.entries.len() <= COUNT);
        assert!(
            c.entries
                .values()
                .map(|v| v.value.bytes.len())
                .sum::<usize>()
                <= BUDGET
        );
    }
}

pub fn invalidate_all() {
    let mut c = cache().lock().unwrap();
    c.entries.clear();
    c.epochs.clear();
}
