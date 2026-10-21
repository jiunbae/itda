//! When a viewer wants the next framebuffer update, shared by the "Share this PC" server and
//! the RDP bridge. Classic RFB answers one FramebufferUpdateRequest per update, costing a
//! round trip per frame; with Continuous Updates the server pushes changes as they happen,
//! and Fence acknowledgements keep it from running ahead of a slow link.
//!
//! Limits: the window counts updates, not the socket. A write to a viewer that stopped
//! reading still blocks its pump (and, in the RDP bridge, RDP input), and ACK_TIMEOUT is only
//! checked between writes, so it doesn't bound how long that takes to notice.

use std::collections::VecDeque;
use std::io::{self, Read};
use std::time::{Duration, Instant};

pub const ENC_CONTINUOUS: i32 = -313;
pub const ENC_FENCE: i32 = -312;
/// Client and server message type of EnableContinuousUpdates / EndOfContinuousUpdates.
pub const MSG_CONTINUOUS: u8 = 150;
pub const MSG_FENCE: u8 = 248;
const FENCE_REQUEST: u32 = 1 << 31;
/// Fence flags honoured when answering the viewer's fence: none. (BlockBefore/BlockAfter
/// would need a barrier across the reader thread's input handling.)
const FENCE_SUPPORTED: u32 = 0;

/// Unacknowledged updates allowed on the wire: always one, more while they are small. The
/// byte figure is a soft limit: the update that crosses it still goes out whole. Up to
/// BASE_FRAMES as before; video frames are ~10-20 KB and acks over a tunnel take 70+ ms, so
/// up to MAX_FRAMES while the oldest unacked update is younger than FRESH: the stream keeps
/// its rate through ordinary latency, but a long stall still stops it instead of queueing a
/// backlog the viewer would show late. Once acks have been timed, "young" is the link's
/// quickest recent round trip plus QUEUE_SLACK (within MIN_FRESH..FRESH), so frames don't
/// pile up in the tunnel's buffers well past what the path itself takes.
const BASE_FRAMES: usize = 4;
const MAX_FRAMES: usize = 16;
const FRESH: Duration = Duration::from_millis(150);
const MIN_FRESH: Duration = Duration::from_millis(50);
const QUEUE_SLACK: Duration = Duration::from_millis(30);
/// How long the quickest round trip stands before a newer sample replaces it (the path changes).
const RTT_WINDOW: Duration = Duration::from_secs(10);
const MAX_BYTES: usize = 1 << 20;
/// A viewer that leaves an update unacknowledged this long has stopped reading.
const ACK_TIMEOUT: Duration = Duration::from_secs(30);

/// The client messages this module handles.
#[derive(Debug, PartialEq)]
pub enum Ext {
    Continuous { on: bool, x: u16, y: u16, w: u16, h: u16 },
    Fence { flags: u32, payload: Vec<u8> },
}

/// Read the body of client message `t` (MSG_CONTINUOUS or MSG_FENCE), its type byte already read.
pub fn read(t: u8, r: &mut impl Read) -> io::Result<Ext> {
    let u16_at = |b: &[u8], i: usize| u16::from_be_bytes([b[i], b[i + 1]]);
    if t == MSG_CONTINUOUS {
        let mut b = [0u8; 9];
        r.read_exact(&mut b)?;
        return Ok(Ext::Continuous { on: b[0] != 0, x: u16_at(&b, 1), y: u16_at(&b, 3), w: u16_at(&b, 5), h: u16_at(&b, 7) });
    }
    let mut b = [0u8; 8];
    r.read_exact(&mut b)?;
    let mut payload = vec![0u8; b[7] as usize];
    r.read_exact(&mut payload)?;
    if payload.len() > 64 {
        return Err(io::Error::other("fence payload too long"));
    }
    Ok(Ext::Fence { flags: u32::from_be_bytes([b[3], b[4], b[5], b[6]]), payload })
}

fn fence(out: &mut Vec<u8>, flags: u32, payload: &[u8]) {
    out.extend_from_slice(&[MSG_FENCE, 0, 0, 0]);
    out.extend_from_slice(&flags.to_be_bytes());
    out.push(payload.len() as u8);
    out.extend_from_slice(payload);
}

#[derive(Default)]
pub struct Demand {
    /// A FramebufferUpdateRequest waiting for its answer: Some(incremental).
    want: Option<bool>,
    /// Told the viewer we do Continuous Updates (sent EndOfContinuousUpdates once).
    offered: bool,
    /// Continuous Updates are on, for this region (x, y, w, h).
    region: Option<(usize, usize, usize, usize)>,
    seq: u32,
    /// Fenced updates not yet acknowledged: (sequence, bytes, sent at).
    inflight: VecDeque<(u32, usize, Instant)>,
    /// The quickest recent round trip of a fenced update, and when it was measured.
    rtt: Option<(Duration, Instant)>,
}

impl Demand {
    /// SetEncodings. The first time the viewer can do Continuous Updates with Fence, offer them.
    pub fn encodings(&mut self, encs: &[i32], out: &mut Vec<u8>) {
        if !self.offered && encs.contains(&ENC_CONTINUOUS) && encs.contains(&ENC_FENCE) {
            self.offered = true;
            out.push(MSG_CONTINUOUS);
        }
    }

    /// FramebufferUpdateRequest. While continuous, only a full (non-incremental) one matters.
    pub fn request(&mut self, incremental: bool) {
        if self.region.is_some() && incremental {
            return;
        }
        self.want = Some(self.want.unwrap_or(true) && incremental);
    }

    /// EnableContinuousUpdates or a Fence from the viewer; replies go to `out`. True when
    /// the update region changed so that the w×h framebuffer's history must be dropped:
    /// tiles outside the old region were never sent.
    pub fn ext(&mut self, m: Ext, fw: usize, fh: usize, out: &mut Vec<u8>) -> bool {
        let before = (self.region, self.whole(fw, fh));
        match m {
            Ext::Continuous { on: true, x, y, w, h } if self.offered => {
                self.region = Some((x as usize, y as usize, w as usize, h as usize));
            }
            Ext::Continuous { .. } => {
                // Off (or never offered): confirm that no more pushed updates follow.
                self.region = None;
                self.inflight.clear();
                out.push(MSG_CONTINUOUS);
            }
            Ext::Fence { flags, payload } if flags & FENCE_REQUEST != 0 => fence(out, flags & FENCE_SUPPORTED, &payload),
            Ext::Fence { payload, .. } => {
                if let Ok(b) = <[u8; 4]>::try_from(&payload[..]) {
                    let seq = u32::from_be_bytes(b);
                    if let Some(i) = self.inflight.iter().position(|&(s, ..)| s == seq) {
                        let rtt = self.inflight[i].2.elapsed();
                        crate::trace::add("ack", rtt.as_secs_f32() * 1000.0);
                        if self.rtt.is_none_or(|(best, at)| rtt <= best || at.elapsed() > RTT_WINDOW) {
                            self.rtt = Some((rtt, Instant::now()));
                        }
                        // A fence answers everything sent before it too.
                        self.inflight.drain(..=i);
                    }
                }
                return false;
            }
        }
        before.0 != self.region && !(before.1 && self.whole(fw, fh))
    }

    /// The viewer stopped acknowledging pushed updates.
    pub fn stalled(&self) -> bool {
        self.inflight.front().is_some_and(|&(.., at)| at.elapsed() > ACK_TIMEOUT)
    }

    /// Should an update go out now, and is it incremental? Pushed updates wait while the
    /// link still holds too many unacknowledged ones.
    pub fn due(&self) -> Option<bool> {
        if self.want == Some(false) {
            return Some(false);
        }
        if self.region.is_some() {
            let bytes: usize = self.inflight.iter().map(|&(_, b, _)| b).sum();
            let fresh = self.rtt.map_or(FRESH, |(best, _)| (best + QUEUE_SLACK).clamp(MIN_FRESH, FRESH));
            let young = self.inflight.front().is_some_and(|&(.., at)| at.elapsed() < fresh);
            let n = self.inflight.len();
            let room = n == 0 || (bytes < MAX_BYTES && (n < BASE_FRAMES || (n < MAX_FRAMES && young)));
            return room.then_some(true);
        }
        self.want
    }

    /// Does the update cover the tile at (x, y, w, h)? Everything for a full request or
    /// classic updates; while continuous, tiles meeting the region.
    pub fn covers(&self, x: usize, y: usize, w: usize, h: usize) -> bool {
        self.want == Some(false)
            || self.region.is_none_or(|(rx, ry, rw, rh)| rw > 0 && rh > 0 && w > 0 && h > 0 && x < rx + rw && rx < x + w && y < ry + rh && ry < y + h)
    }

    /// Does the update region take in the whole w×h framebuffer? (Always, unless continuous.)
    pub fn whole(&self, w: usize, h: usize) -> bool {
        self.region.is_none_or(|(x, y, rw, rh)| x == 0 && y == 0 && rw >= w && rh >= h)
    }

    #[cfg(test)]
    fn continuous(&self) -> bool {
        self.region.is_some()
    }

    /// An update of `bytes` is about to be queued, in order, for the viewer (from then on it
    /// is part of the stream). While continuous, follow it with a fence to count it; the ack
    /// timeout runs from here, so it includes time spent in the send queue.
    pub fn sent(&mut self, bytes: usize, out: &mut Vec<u8>) {
        self.want = None;
        if self.region.is_some() {
            self.seq = self.seq.wrapping_add(1);
            fence(out, FENCE_REQUEST, &self.seq.to_be_bytes());
            self.inflight.push_back((self.seq, bytes, Instant::now()));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const BOTH: [i32; 3] = [16, ENC_CONTINUOUS, ENC_FENCE];

    fn on(d: &mut Demand) {
        let mut out = Vec::new();
        d.encodings(&BOTH, &mut out);
        assert_eq!(out, [MSG_CONTINUOUS]);
        d.ext(Ext::Continuous { on: true, x: 0, y: 0, w: 100, h: 50 }, 640, 480, &mut out);
    }

    /// The fence that follows update `n` (the 4-byte sequence payload).
    fn ack(d: &mut Demand, n: u32) {
        assert!(!d.ext(Ext::Fence { flags: 0, payload: n.to_be_bytes().to_vec() }, 640, 480, &mut Vec::new()), "fences keep history");
    }

    #[test]
    fn classic_request_answer() {
        let mut d = Demand::default();
        let mut out = Vec::new();
        d.encodings(&[16, ENC_CONTINUOUS], &mut out); // no Fence: nothing offered
        assert!(out.is_empty());
        assert_eq!(d.due(), None);
        d.request(true);
        assert_eq!(d.due(), Some(true));
        d.request(false); // a full request wins over an incremental one
        assert_eq!(d.due(), Some(false));
        d.sent(10, &mut out);
        assert!(out.is_empty(), "no fences outside continuous mode");
        assert_eq!(d.due(), None);
        // Enabling without having been offered is refused with EndOfContinuousUpdates.
        d.ext(Ext::Continuous { on: true, x: 0, y: 0, w: 1, h: 1 }, 640, 480, &mut out);
        assert_eq!(out, [MSG_CONTINUOUS]);
        assert!(!d.continuous());
    }

    #[test]
    fn offered_once() {
        let mut d = Demand::default();
        let mut out = Vec::new();
        d.encodings(&BOTH, &mut out);
        d.encodings(&BOTH, &mut out);
        assert_eq!(out, [MSG_CONTINUOUS]);
    }

    #[test]
    fn continuous_pushes_within_a_window() {
        let mut d = Demand::default();
        on(&mut d);
        assert_eq!(d.due(), Some(true), "pushes without a request");
        d.request(true); // incremental requests are ignored
        let mut out = Vec::new();
        for n in 1..=MAX_FRAMES as u32 {
            assert_eq!(d.due(), Some(true));
            out.clear();
            d.sent(1000, &mut out);
            let mut want = vec![MSG_FENCE, 0, 0, 0, 0x80, 0, 0, 0, 4];
            want.extend_from_slice(&n.to_be_bytes());
            assert_eq!(out, want);
        }
        assert_eq!(d.due(), None, "window full");
        ack(&mut d, 2); // acknowledges 1 and 2
        assert_eq!(d.due(), Some(true));
        ack(&mut d, 99); // unknown: ignored
        ack(&mut d, MAX_FRAMES as u32);
        assert!(d.inflight.is_empty());
    }

    #[test]
    fn window_grows_only_while_acks_are_recent() {
        let mut d = Demand::default();
        on(&mut d);
        for _ in 0..BASE_FRAMES {
            d.sent(1000, &mut Vec::new());
        }
        assert_eq!(d.due(), Some(true), "young: beyond the base window");
        std::thread::sleep(FRESH + Duration::from_millis(20));
        assert_eq!(d.due(), None, "a stalled link stops at the base window");
        ack(&mut d, 1);
        assert_eq!(d.due(), Some(true), "back under the base window");
    }

    #[test]
    fn window_follows_the_round_trip() {
        let mut d = Demand::default();
        on(&mut d);
        d.sent(1000, &mut Vec::new());
        ack(&mut d, 1); // a quick link: "young" shrinks to MIN_FRESH
        for _ in 0..BASE_FRAMES {
            d.sent(1000, &mut Vec::new());
        }
        assert_eq!(d.due(), Some(true), "young");
        std::thread::sleep(MIN_FRESH + Duration::from_millis(20));
        assert_eq!(d.due(), None, "queued well past the round trip");
        ack(&mut d, 2);
        assert_eq!(d.due(), Some(true), "back under the base window");
    }

    #[test]
    fn one_big_update_at_a_time() {
        let mut d = Demand::default();
        on(&mut d);
        d.sent(MAX_BYTES, &mut Vec::new());
        assert_eq!(d.due(), None);
        // A full refresh is still honoured.
        d.request(false);
        assert_eq!(d.due(), Some(false));
        ack(&mut d, 1);
        d.sent(10, &mut Vec::new());
        assert_eq!(d.due(), Some(true));
    }

    #[test]
    fn region_and_disable() {
        let mut d = Demand::default();
        assert!(d.covers(500, 500, 64, 64));
        on(&mut d);
        assert!(d.covers(64, 0, 64, 64));
        assert!(!d.covers(128, 0, 64, 64));
        assert!(!d.covers(0, 64, 64, 64));
        // An empty region covers nothing.
        d.ext(Ext::Continuous { on: true, x: 10, y: 10, w: 0, h: 0 }, 640, 480, &mut Vec::new());
        assert!(!d.covers(0, 0, 64, 64));
        d.ext(Ext::Continuous { on: true, x: 10, y: 10, w: 5, h: 0 }, 640, 480, &mut Vec::new());
        assert!(!d.covers(0, 0, 64, 64));
        d.sent(10, &mut Vec::new());
        let mut out = Vec::new();
        assert!(d.ext(Ext::Continuous { on: false, x: 0, y: 0, w: 0, h: 0 }, 640, 480, &mut out), "the rest was never sent");
        assert_eq!(out, [MSG_CONTINUOUS]);
        assert!(!d.continuous() && d.inflight.is_empty());
        assert_eq!(d.due(), None, "back to waiting for requests");
        assert!(d.covers(500, 500, 64, 64));
    }

    /// Only a region change that leaves part of the framebuffer unsent drops history.
    #[test]
    fn region_changes() {
        let mut d = Demand::default();
        let mut out = Vec::new();
        d.encodings(&BOTH, &mut out);
        let set = |d: &mut Demand, w: u16| d.ext(Ext::Continuous { on: true, x: 0, y: 0, w, h: 480 }, 640, 480, &mut Vec::new());
        assert!(!set(&mut d, 640), "classic → whole framebuffer");
        assert!(!set(&mut d, 640), "same region again");
        assert!(set(&mut d, 320), "whole → half");
        assert!(!set(&mut d, 320), "same half again");
        d.sent(10, &mut out);
        assert!(!d.ext(Ext::Fence { flags: 0, payload: 1u32.to_be_bytes().to_vec() }, 640, 480, &mut out));
        assert!(set(&mut d, 640), "half → whole");
    }

    /// A full request covers the whole framebuffer even while continuous for part of it.
    #[test]
    fn full_request_beats_region() {
        let mut d = Demand::default();
        on(&mut d);
        assert!(!d.covers(600, 400, 40, 80));
        d.request(false);
        assert!(d.covers(600, 400, 40, 80));
        d.sent(10, &mut Vec::new());
        assert!(!d.covers(600, 400, 40, 80));
    }

    #[test]
    fn stalls_without_acks() {
        let mut d = Demand::default();
        on(&mut d);
        d.sent(10, &mut Vec::new());
        assert!(!d.stalled());
        d.inflight[0].2 -= ACK_TIMEOUT + Duration::from_secs(1);
        assert!(d.stalled());
        ack(&mut d, 1);
        assert!(!d.stalled());
    }

    #[test]
    fn answers_fence_requests() {
        let mut d = Demand::default();
        let mut out = Vec::new();
        // Request + BlockBefore + SyncNext: answered without flags (none honoured), same payload.
        d.ext(Ext::Fence { flags: FENCE_REQUEST | 1 | 4, payload: vec![7, 8] }, 640, 480, &mut out);
        assert_eq!(out, [MSG_FENCE, 0, 0, 0, 0, 0, 0, 0, 2, 7, 8]);
    }

    #[test]
    fn parses_messages() {
        let mut r: &[u8] = &[1, 0, 1, 0, 2, 0, 3, 0, 4];
        assert_eq!(read(MSG_CONTINUOUS, &mut r).unwrap(), Ext::Continuous { on: true, x: 1, y: 2, w: 3, h: 4 });
        let mut r: &[u8] = &[0, 0, 0, 0x80, 0, 0, 3, 2, 9, 9];
        assert_eq!(read(MSG_FENCE, &mut r).unwrap(), Ext::Fence { flags: 0x8000_0003, payload: vec![9, 9] });
        let mut long = vec![0u8, 0, 0, 0, 0, 0, 0, 65];
        long.extend([0; 65]);
        assert!(read(MSG_FENCE, &mut &long[..]).is_err());
    }
}
