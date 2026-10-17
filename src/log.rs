//! Tiny diagnostics log: %TEMP%\itda.log for the home window (overwritten each run),
//! %TEMP%\itda\session-<pid>.log for each connection (old ones are pruned).

use std::fs::File;
use std::io::Write;
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant, SystemTime};

static LOG: Mutex<Option<File>> = Mutex::new(None);
static T0: OnceLock<Instant> = OnceLock::new();
static PATH: OnceLock<PathBuf> = OnceLock::new();

pub fn path() -> PathBuf {
    PATH.get().cloned().unwrap_or_else(|| std::env::temp_dir().join("itda.log"))
}

pub fn init(kind: &str) {
    T0.get_or_init(Instant::now);
    let p = if kind.is_empty() {
        prune();
        std::env::temp_dir().join("itda.log")
    } else {
        let d = std::env::temp_dir().join("itda");
        let _ = std::fs::create_dir_all(&d);
        d.join(format!("{kind}-{}.log", std::process::id()))
    };
    *LOG.lock().unwrap() = File::create(&p).ok();
    let _ = PATH.set(p);
}

/// Drop session logs older than three days.
fn prune() {
    let Ok(rd) = std::fs::read_dir(std::env::temp_dir().join("itda")) else { return };
    let cutoff = SystemTime::now() - Duration::from_secs(3 * 86400);
    for e in rd.flatten() {
        if e.metadata().and_then(|m| m.modified()).is_ok_and(|t| t < cutoff) {
            let _ = std::fs::remove_file(e.path());
        }
    }
}

pub fn write(s: String) {
    let t = T0.get().map_or(0.0, |t| t.elapsed().as_secs_f64());
    if let Some(f) = LOG.lock().unwrap().as_mut() {
        let _ = writeln!(f, "{t:9.3} {s}");
    }
}

#[macro_export]
macro_rules! log {
    ($($t:tt)*) => { $crate::log::write(format!($($t)*)) };
}
