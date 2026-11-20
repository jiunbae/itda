//! Autorelease pools for Itda's own long-lived threads (macOS).
//!
//! AppKit and ScreenCaptureKit hand back autoreleased objects even to Rust callers: asking
//! for the current cursor's image, the pasteboard's change count, the screen-capture
//! permission. Only the main thread drains a pool on every turn of its run loop. On a
//! thread of our own, without a pool, those objects pile up until the thread ends: a
//! viewer's pump grew by about 120 KB a frame, tens of gigabytes over an hour of sharing,
//! all freed at once when the viewer left but leaving the memory fragmented and swapped.
//!
//! A `Pool` held for one turn of a loop drains whatever that turn autoreleased when it is
//! dropped (also on `continue`, `?` and `return`). Pools nest, so a function that makes
//! its own stays correct inside one. Elsewhere it compiles to nothing.

pub struct Pool {
    #[cfg(target_os = "macos")]
    token: *mut std::ffi::c_void,
}

impl Pool {
    #[inline]
    pub fn new() -> Pool {
        Pool {
            #[cfg(target_os = "macos")]
            token: unsafe { objc2::ffi::objc_autoreleasePoolPush() },
        }
    }
}

impl Drop for Pool {
    #[inline]
    fn drop(&mut self) {
        #[cfg(target_os = "macos")]
        unsafe {
            objc2::ffi::objc_autoreleasePoolPop(self.token)
        }
    }
}

/// Give memory held free back to the system (macOS), e.g. once a viewer has left: what a
/// burst freed otherwise stays counted against Itda, and is swapped out under pressure rather
/// than dropped. Big blocks (frames) wait in Itda's own cache, everything else in malloc's.
pub fn release_free_memory() {
    #[cfg(target_os = "macos")]
    {
        crate::bigalloc::flush();
        unsafe extern "C" {
            fn malloc_zone_pressure_relief(zone: *mut std::ffi::c_void, goal: usize) -> usize;
        }
        // (No zone: every zone; goal 0: as much as it can.)
        unsafe { malloc_zone_pressure_relief(std::ptr::null_mut(), 0) };
    }
}

/// Run `f` in a pool of its own.
#[inline]
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
pub fn run<T>(f: impl FnOnce() -> T) -> T {
    let _pool = Pool::new();
    f()
}

#[cfg(all(test, target_os = "macos"))]
mod tests {
    use super::*;
    use objc2::rc::{Retained, Weak};
    use objc2_foundation::NSObject;

    /// What a thread autoreleases in a turn is gone when the turn's pool drops, not at the
    /// thread's end (a viewer's pump lives as long as the viewer).
    #[test]
    fn a_turn_drains_what_it_autoreleased() {
        std::thread::spawn(|| {
            let weak = {
                let _pool = Pool::new();
                let obj = NSObject::new();
                let weak = Weak::from_retained(&obj);
                let _ = Retained::autorelease_ptr(obj); // only the pool holds it now
                assert!(weak.load().is_some(), "alive until the pool drains");
                weak
            };
            assert!(weak.load().is_none(), "the turn's pool released it");
            // Pools nest: an inner one drains only its own.
            let _outer = Pool::new();
            let keep = NSObject::new();
            let kept = Weak::from_retained(&keep);
            let _ = Retained::autorelease_ptr(keep);
            run(|| ());
            assert!(kept.load().is_some(), "the inner pool left the outer one's object");
        })
        .join()
        .unwrap();
    }

    /// A viewer's 4K frames (about 33 MB each), once freed, stop counting against Itda when it
    /// hands free memory back. Until then malloc keeps them cached as "Malloc Large (empty)".
    #[test]
    fn freed_frames_leave_the_footprint() {
        unsafe extern "C" {
            fn proc_pid_rusage(pid: i32, flavor: i32, buffer: *mut [u64; 64]) -> i32;
        }
        let footprint = || {
            let mut b = [0u64; 64];
            assert_eq!(unsafe { proc_pid_rusage(std::process::id() as i32, 2, &mut b) }, 0);
            b[9] >> 20
        };
        release_free_memory();
        let base = footprint();
        let frames: Vec<Vec<u32>> = (0..4).map(|i| vec![i + 1; 3840 * 2160]).collect();
        // (No check against `base`: tests alongside free their own memory meanwhile.)
        let with = footprint();
        drop(frames);
        let freed = footprint();
        release_free_memory();
        let after = footprint();
        println!("base {base} MB, with frames {with} MB, freed {freed} MB, returned {after} MB");
        // (Other tests run alongside: judge by what this release itself dropped.)
        assert!(after + 100 < freed, "base {base} MB, freed {freed} MB, after returning {after} MB");
    }
}
