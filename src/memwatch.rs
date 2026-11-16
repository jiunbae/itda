//! Itda keeps an eye on its own memory. Once a minute it reads what the system counts against
//! it: the physical footprint on macOS, the private commit on Windows. It writes that to the
//! log when it reaches a new 256 MB step and once an hour. Past 1 GiB it says so loudly, along
//! with how many viewers are watching, and hands the memory malloc holds free back to the
//! system.
//!
//! Nothing this app does should need a gigabyte. A slow build-up, like the autorelease pile-up
//! that took a sharing Mac to 101 GiB, then leaves a trail in itda.log: when it began, how
//! fast it grew, and whether a viewer was connected. It doesn't wait for someone to notice the
//! Mac slowing down.

use std::time::{Duration, Instant};

const EVERY: Duration = Duration::from_secs(60);
const STEP_MB: u64 = 256;
const HOURLY: Duration = Duration::from_secs(3600);
/// Past this the log says it's too much (and malloc's free pages go back to the system).
pub const TOO_MUCH_MB: u64 = 1024;

/// What the system counts against this process, in MB.
pub fn footprint_mb() -> Option<u64> {
    #[cfg(target_os = "macos")]
    {
        unsafe extern "C" {
            fn proc_pid_rusage(pid: i32, flavor: i32, buffer: *mut [u64; 64]) -> i32;
        }
        let mut b = [0u64; 64];
        // RUSAGE_INFO_V2: after the 16-byte uuid, ri_phys_footprint is the eighth u64.
        (unsafe { proc_pid_rusage(std::process::id() as i32, 2, &mut b) } == 0).then(|| b[9] >> 20)
    }
    #[cfg(windows)]
    {
        #[repr(C)]
        #[derive(Default)]
        struct Counters {
            cb: u32,
            page_faults: u32,
            peak_working_set: usize,
            working_set: usize,
            quota_peak_paged: usize,
            quota_paged: usize,
            quota_peak_nonpaged: usize,
            quota_nonpaged: usize,
            pagefile: usize,
            peak_pagefile: usize,
            private: usize,
        }
        #[link(name = "kernel32")]
        unsafe extern "system" {
            fn K32GetProcessMemoryInfo(process: isize, counters: *mut Counters, cb: u32) -> i32;
        }
        let mut c = Counters { cb: size_of::<Counters>() as u32, ..Default::default() };
        // (-1: this process.)
        (unsafe { K32GetProcessMemoryInfo(-1, &mut c, c.cb) } != 0).then(|| c.private as u64 >> 20)
    }
}

/// Why a reading goes to the log, if it does.
#[derive(Debug, PartialEq)]
enum Note {
    First,
    Step,
    Hourly,
    TooMuch,
}

#[derive(Default)]
struct Watch {
    /// The highest step reached (MB, a multiple of STEP_MB), and when the log last had a line.
    step: Option<u64>,
    last: Option<Instant>,
    too_much: bool,
}

impl Watch {
    fn see(&mut self, mb: u64, now: Instant) -> Option<Note> {
        let step = mb / STEP_MB * STEP_MB;
        let note = if self.last.is_none() {
            Some(Note::First)
        } else if mb >= TOO_MUCH_MB && !self.too_much {
            Some(Note::TooMuch)
        } else if self.step.is_some_and(|s| step > s) {
            Some(Note::Step)
        } else if self.last.is_some_and(|t| now.duration_since(t) >= HOURLY) {
            Some(Note::Hourly)
        } else {
            None
        };
        // Back under the limit: the next time past it is news again.
        self.too_much = mb >= TOO_MUCH_MB;
        self.step = Some(self.step.map_or(step, |s| s.max(step)));
        if note.is_some() {
            self.last = Some(now);
        }
        note
    }
}

/// Watch for as long as the app runs.
pub fn start() {
    std::thread::Builder::new()
        .name("memwatch".into())
        .spawn(|| {
            let mut w = Watch::default();
            let mut peak = 0;
            loop {
                if let Some(mb) = footprint_mb() {
                    peak = peak.max(mb);
                    let viewers = crate::server::viewers().len();
                    match w.see(mb, Instant::now()) {
                        Some(Note::TooMuch) => {
                            crate::log!("memory: {mb} MB, too much for Itda (peak {peak} MB, {viewers} viewers): returning free memory");
                            crate::pool::release_free_memory();
                        }
                        Some(why) => crate::log!("memory: {mb} MB ({why:?}, peak {peak} MB, {viewers} viewers)"),
                        None => {}
                    }
                }
                std::thread::sleep(EVERY);
            }
        })
        .ok();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn this_process_has_a_footprint() {
        let mb = footprint_mb().expect("readable");
        assert!(mb >= 1 && mb < 64 << 10, "{mb} MB");
    }

    #[test]
    fn logs_the_first_reading_new_steps_an_hour_and_too_much() {
        let t = Instant::now();
        let mut w = Watch::default();
        assert_eq!(w.see(40, t), Some(Note::First));
        assert_eq!(w.see(60, t + EVERY), None, "same step");
        assert_eq!(w.see(300, t + EVERY * 2), Some(Note::Step));
        assert_eq!(w.see(200, t + EVERY * 3), None, "lower again");
        assert_eq!(w.see(290, t + EVERY * 4), None, "a step already seen");
        assert_eq!(w.see(290, t + EVERY * 2 + HOURLY), Some(Note::Hourly));
        assert_eq!(w.see(1100, t + EVERY * 3 + HOURLY), Some(Note::TooMuch));
        assert_eq!(w.see(1150, t + EVERY * 4 + HOURLY), None, "said once while it stays high");
        assert_eq!(w.see(500, t + EVERY * 5 + HOURLY), None);
        assert_eq!(w.see(1200, t + EVERY * 6 + HOURLY), Some(Note::TooMuch), "past it again");
    }
}
