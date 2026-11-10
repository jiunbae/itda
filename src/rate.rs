//! Adaptive video bitrate for one viewer of the share server.
//!
//! The H.264 encoders start at a bitrate chosen from the screen size, which a LAN carries
//! easily but a slow tunnel or a phone on cellular may not. Then each update waits longer
//! for its acknowledgement, the in-flight window (demand.rs) fills, and frames are skipped:
//! the picture stays sharp but turns into a slideshow. Lowering the bitrate instead keeps
//! the frame rate and lets the lossless repaint (server.rs) sharpen the picture once it
//! stands still.
//!
//! Congestion signs: acknowledgements taking well longer than the link's quiet round trip
//! (queues growing somewhere), or the pump holding a frame because the window or the send
//! queue was full. Back off multiplicatively, at most once per BACKOFF_GAP so the last
//! change can take effect; with no sign for PROBE_AFTER, step back up towards the start.

use std::time::{Duration, Instant};

/// How often the controller decides.
const TICK: Duration = Duration::from_millis(250);
/// Least time between two decreases: what's already queued must drain first.
const BACKOFF_GAP: Duration = Duration::from_millis(600);
/// Quiet time before stepping up, and between steps up.
const PROBE_AFTER: Duration = Duration::from_millis(1500);
/// The quiet round trip is the least ack delay seen recently; it's re-learned this often
/// so a route change (Wi-Fi to cellular, a new Tailscale path) isn't judged by an old one.
const BASE_WINDOW: Duration = Duration::from_secs(10);
/// An ack this much slower than the quiet round trip (plus the frame's own transfer) means
/// a queue is building.
const SLACK: Duration = Duration::from_millis(40);
const DOWN: f32 = 0.7;
const UP: f32 = 1.15;
/// Lowest bitrate, as a share of the starting one, and in absolute terms.
const FLOOR_SHARE: u32 = 10;
const FLOOR: u32 = 600_000;

pub struct Rate {
    max: u32,
    min: u32,
    cur: u32,
    /// Smoothed ack delay of this tick's samples (None: no acks yet).
    srtt: Option<Duration>,
    /// Least ack delay in the current and the previous BASE_WINDOW.
    base: Option<Duration>,
    window_min: Option<Duration>,
    window_start: Instant,
    /// Frames held back since the last tick.
    held: u32,
    last_tick: Instant,
    last_down: Option<Instant>,
    last_change: Instant,
}

impl Rate {
    /// Starting (and highest) bitrate.
    pub fn new(max: u32, now: Instant) -> Rate {
        let min = (max / FLOOR_SHARE).max(FLOOR).min(max);
        Rate { max, min, cur: max, srtt: None, base: None, window_min: None, window_start: now, held: 0, last_tick: now, last_down: None, last_change: now }
    }

    pub fn bitrate(&self) -> u32 {
        self.cur
    }

    /// A new encoder whose own starting bitrate is `max` (the screen size changed): keep how
    /// much the link has shown it can take.
    pub fn rebase(&mut self, max: u32) {
        let share = self.cur as f64 / self.max as f64;
        self.max = max;
        self.min = (max / FLOOR_SHARE).max(FLOOR).min(max);
        self.cur = ((max as f64 * share) as u32).clamp(self.min, self.max);
    }

    /// An update was acknowledged `delay` after it was queued.
    pub fn ack(&mut self, delay: Duration) {
        self.srtt = Some(match self.srtt {
            Some(s) => (s * 7 + delay) / 8,
            None => delay,
        });
        self.window_min = Some(self.window_min.map_or(delay, |m| m.min(delay)));
        self.base = Some(self.base.map_or(delay, |b| b.min(delay)));
    }

    /// A changed picture couldn't go out: the window or the send queue was full.
    pub fn held(&mut self) {
        self.held += 1;
    }

    /// Decide; Some(new bitrate) when it changed.
    pub fn tick(&mut self, now: Instant) -> Option<u32> {
        if now.duration_since(self.last_tick) < TICK {
            return None;
        }
        self.last_tick = now;
        if now.duration_since(self.window_start) >= BASE_WINDOW {
            // Forget older minima: the base is now this window's least delay.
            self.base = self.window_min.or(self.base);
            self.window_min = None;
            self.window_start = now;
        }
        let slow = matches!((self.srtt, self.base), (Some(s), Some(b)) if s > b + b / 2 + SLACK);
        let congested = slow || self.held > 0;
        self.held = 0;
        let before = self.cur;
        if congested {
            if self.last_down.is_none_or(|t| now.duration_since(t) >= BACKOFF_GAP) {
                self.cur = ((self.cur as f32 * DOWN) as u32).max(self.min);
                self.last_down = Some(now);
                self.last_change = now;
            }
        } else if self.cur < self.max && now.duration_since(self.last_change) >= PROBE_AFTER && self.last_down.is_none_or(|t| now.duration_since(t) >= PROBE_AFTER) {
            self.cur = ((self.cur as f32 * UP) as u32).min(self.max);
            self.last_change = now;
        }
        (self.cur != before).then_some(self.cur)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MS: Duration = Duration::from_millis(1);

    /// Run `secs` seconds of 60 acks a second at `delay`, ticking as the pump would.
    fn run(r: &mut Rate, t: &mut Instant, secs: u32, delay: Duration, held: bool) -> Vec<u32> {
        let mut changes = Vec::new();
        for _ in 0..secs * 60 {
            *t += Duration::from_micros(16_667);
            r.ack(delay);
            if held {
                r.held();
            }
            changes.extend(r.tick(*t));
        }
        changes
    }

    #[test]
    fn steady_link_keeps_the_starting_bitrate() {
        let mut t = Instant::now();
        let mut r = Rate::new(20_000_000, t);
        assert!(run(&mut r, &mut t, 10, 30 * MS, false).is_empty());
        assert_eq!(r.bitrate(), 20_000_000);
    }

    #[test]
    fn growing_delay_backs_off_then_recovers() {
        let mut t = Instant::now();
        let mut r = Rate::new(20_000_000, t);
        run(&mut r, &mut t, 2, 30 * MS, false);
        let down = run(&mut r, &mut t, 3, 200 * MS, false);
        assert!(down.len() >= 3, "{down:?}");
        assert!(down.windows(2).all(|w| w[1] < w[0]));
        let low = r.bitrate();
        assert!(low < 8_000_000, "{low}");
        // Backs off at most once per BACKOFF_GAP.
        assert!(down.len() as u32 <= 3000 / BACKOFF_GAP.as_millis() as u32 + 1);
        // Queues drain: delay back to normal, so it steps back up, never past the start.
        let up = run(&mut r, &mut t, 30, 30 * MS, false);
        assert!(up.windows(2).all(|w| w[1] > w[0]), "{up:?}");
        assert_eq!(r.bitrate(), 20_000_000);
    }

    #[test]
    fn held_frames_count_as_congestion_and_the_floor_holds() {
        let mut t = Instant::now();
        let mut r = Rate::new(20_000_000, t);
        run(&mut r, &mut t, 20, 30 * MS, true);
        assert_eq!(r.bitrate(), 2_000_000, "a tenth of the start");
        let mut t2 = Instant::now();
        let mut small = Rate::new(4_000_000, t2);
        run(&mut small, &mut t2, 20, 30 * MS, true);
        assert_eq!(small.bitrate(), FLOOR);
    }

    #[test]
    fn a_new_route_is_judged_by_its_own_round_trip() {
        let mut t = Instant::now();
        let mut r = Rate::new(20_000_000, t);
        run(&mut r, &mut t, 3, 5 * MS, false);
        // Now 80 ms away but not congested: it may back off at first, but once the base is
        // re-learned it climbs back to the start.
        run(&mut r, &mut t, 60, 80 * MS, false);
        assert_eq!(r.bitrate(), 20_000_000);
    }

    #[test]
    fn rebase_keeps_the_learned_share() {
        let mut t = Instant::now();
        let mut r = Rate::new(20_000_000, t);
        run(&mut r, &mut t, 2, 30 * MS, true);
        let share = r.bitrate() as f64 / 20_000_000.0;
        r.rebase(10_000_000);
        assert!(((r.bitrate() as f64 / 10_000_000.0) - share).abs() < 0.01);
    }
}
