//! `ITDA_HP_TRACE=1`: where a high-performance frame spends its time on this Mac, logged as
//! p50/p95 per stage every 5 s (milliseconds). Off by default: then a flag check per stage.
//! Stages overlap (copy and encode are inside update) and cover different frames, so their
//! quantiles cannot be added up into a frame's age.

use std::collections::BTreeMap;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

const EVERY: Duration = Duration::from_secs(5);

struct Stats {
    since: Instant,
    stages: BTreeMap<&'static str, Vec<f32>>,
}

static STATS: Mutex<Option<Stats>> = Mutex::new(None);

pub fn on() -> bool {
    static ON: OnceLock<bool> = OnceLock::new();
    *ON.get_or_init(|| std::env::var_os("ITDA_HP_TRACE").is_some_and(|v| v == "1"))
}

/// Count `ms` toward `stage`.
pub fn add(stage: &'static str, ms: f32) {
    if !on() {
        return;
    }
    let done = {
        let mut g = STATS.lock().unwrap();
        let s = g.get_or_insert_with(|| Stats { since: Instant::now(), stages: BTreeMap::new() });
        s.stages.entry(stage).or_default().push(ms);
        if s.since.elapsed() >= EVERY { g.take() } else { None }
    };
    // (Summarised outside the lock: the capture callback adds samples too.)
    if let Some(mut s) = done {
        let line: Vec<String> = s.stages.iter_mut().map(|(k, v)| {
            v.sort_by(f32::total_cmp);
            let q = |p: f32| v[((v.len() - 1) as f32 * p).round() as usize];
            format!("{k} n={} p50={:.1} p95={:.1}", v.len(), q(0.5), q(0.95))
        }).collect();
        crate::log!("hp trace: {}", line.join(" | "));
    }
}

pub fn since(stage: &'static str, t: Instant) {
    if on() {
        add(stage, t.elapsed().as_secs_f32() * 1000.0);
    }
}
