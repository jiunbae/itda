//! Big blocks (a 4K frame is about 33 MB) bypass malloc on macOS, so Itda can hand them back
//! the moment a viewer leaves.
//!
//! malloc keeps big blocks it has freed in a cache of its own ("Malloc Large (empty)" in
//! vmmap) and gives them back when it decides to: seconds later, or hours later on a Mac with
//! nothing else going on. `malloc_zone_pressure_relief` doesn't reach that cache. A sharing
//! Mac kept 127 MB of a 7-second viewer's frames for hours.
//!
//! Here big blocks are mapped directly. Freed ones wait in a small cache, so a viewer's steady
//! stream of frames reuses them exactly as it did malloc's. `flush` unmaps what waits there.

use std::alloc::{GlobalAlloc, Layout, System};
use std::ptr::null_mut;
use std::sync::atomic::{AtomicBool, Ordering::{Acquire, Relaxed, Release}};

/// From here up a block is mapped (a 1080p frame is 8.3 MB).
const BIG: usize = 8 << 20;
/// Mapped memory is page aligned; asking for more isn't something Itda does.
const MAX_ALIGN: usize = 4096;
/// Freed big blocks kept for reuse: at most this many, and this much in all.
const SLOTS: usize = 6;
const KEEP: usize = 256 << 20;

pub struct Alloc;

struct Cache {
    blocks: [(usize, usize); SLOTS], // (address, length); 0: empty
}

static LOCK: AtomicBool = AtomicBool::new(false);
static mut CACHE: Cache = Cache { blocks: [(0, 0); SLOTS] };

/// The cache, for a moment. (No std Mutex: it may allocate, and this is the allocator.)
fn with_cache<T>(f: impl FnOnce(&mut Cache) -> T) -> T {
    while LOCK.compare_exchange_weak(false, true, Acquire, Relaxed).is_err() {
        std::hint::spin_loop();
    }
    #[allow(static_mut_refs)]
    let r = f(unsafe { &mut CACHE });
    LOCK.store(false, Release);
    r
}

unsafe extern "C" {
    fn mmap(addr: *mut u8, len: usize, prot: i32, flags: i32, fd: i32, off: i64) -> *mut u8;
    fn munmap(addr: *mut u8, len: usize) -> i32;
    fn getpagesize() -> i32;
}
const PROT_READ_WRITE: i32 = 0x1 | 0x2;
const MAP_PRIVATE_ANON: i32 = 0x0002 | 0x1000;

fn big(l: &Layout) -> bool {
    l.size() >= BIG && l.align() <= MAX_ALIGN
}

fn pages(size: usize) -> usize {
    let page = unsafe { getpagesize() } as usize;
    size.div_ceil(page) * page
}

/// A block of `len` from the cache, or a fresh mapping (already zeroed: `fresh` is true).
unsafe fn take(len: usize) -> (*mut u8, bool) {
    let cached = with_cache(|c| {
        c.blocks.iter_mut().find(|b| b.1 == len).map(|b| std::mem::take(b).0)
    });
    if let Some(addr) = cached {
        return (addr as *mut u8, false);
    }
    let p = unsafe { mmap(null_mut(), len, PROT_READ_WRITE, MAP_PRIVATE_ANON, -1, 0) };
    if p as isize == -1 { (null_mut(), true) } else { (p, true) }
}

unsafe fn put(p: *mut u8, len: usize) {
    let kept = with_cache(|c| {
        let total: usize = c.blocks.iter().map(|b| b.1).sum();
        if total + len > KEEP {
            return false;
        }
        match c.blocks.iter_mut().find(|b| b.0 == 0) {
            Some(slot) => {
                *slot = (p as usize, len);
                true
            }
            None => false,
        }
    });
    if !kept {
        unsafe { munmap(p, len) };
    }
}

/// Give the big blocks waiting for reuse back to the system. Returns how many bytes.
pub fn flush() -> usize {
    let blocks = with_cache(|c| std::mem::replace(&mut c.blocks, [(0, 0); SLOTS]));
    let mut n = 0;
    for (addr, len) in blocks {
        if addr != 0 {
            unsafe { munmap(addr as *mut u8, len) };
            n += len;
        }
    }
    n
}

unsafe impl GlobalAlloc for Alloc {
    unsafe fn alloc(&self, l: Layout) -> *mut u8 {
        if big(&l) { unsafe { take(pages(l.size())).0 } } else { unsafe { System.alloc(l) } }
    }

    unsafe fn alloc_zeroed(&self, l: Layout) -> *mut u8 {
        if !big(&l) {
            return unsafe { System.alloc_zeroed(l) };
        }
        let (p, fresh) = unsafe { take(pages(l.size())) };
        if !fresh {
            unsafe { p.write_bytes(0, l.size()) };
        }
        p
    }

    unsafe fn dealloc(&self, p: *mut u8, l: Layout) {
        if big(&l) { unsafe { put(p, pages(l.size())) } } else { unsafe { System.dealloc(p, l) } }
    }

    unsafe fn realloc(&self, p: *mut u8, l: Layout, size: usize) -> *mut u8 {
        let new = unsafe { Layout::from_size_align_unchecked(size, l.align()) };
        if !big(&l) && !big(&new) {
            return unsafe { System.realloc(p, l, size) };
        }
        if big(&l) && big(&new) && pages(l.size()) == pages(size) {
            return p;
        }
        let q = unsafe { self.alloc(new) };
        if !q.is_null() {
            unsafe {
                std::ptr::copy_nonoverlapping(p, q, l.size().min(size));
                self.dealloc(p, l);
            }
        }
        q
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn big_blocks_come_back_zeroed_and_keep_their_contents() {
        let frame = 3840 * 2160;
        let mut a = vec![7u32; frame];
        a[frame - 1] = 9;
        drop(a);
        // Same size again: likely the block just freed, which must come back zeroed.
        let b: Vec<u32> = vec![0; frame];
        assert!(b.iter().all(|&x| x == 0));
        drop(b);
        // Grow and shrink across the line, keeping the contents.
        let mut v: Vec<u8> = (0..=255).cycle().take(BIG - 1).collect();
        v.reserve_exact(BIG); // to a mapping
        assert_eq!(v[1000], (1000 % 256) as u8);
        v.truncate(10);
        v.shrink_to_fit(); // back to malloc
        assert_eq!(v, (0..10).collect::<Vec<u8>>());
        let _ = flush();
    }
}
