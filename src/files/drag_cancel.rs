//! One Windows source gesture per native window. No OS calls or renderer objects.
use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering::SeqCst};

pub struct Gesture {
    pub window: String,
    pub page: String,
    pub nonce: String,
    pub sequence: u64,
    cancelled: AtomicBool,
    started: AtomicBool,
    pub entered_ole: AtomicBool,
    pub focus_verified: AtomicBool,
    pub escape_seen: AtomicBool,
    pub queries: AtomicU32,
}
impl Gesture {
    pub fn cancel(&self) { self.cancelled.store(true, SeqCst); }
    pub fn cancelled(&self) -> bool { self.cancelled.load(SeqCst) }
    pub fn escape(&self) { self.escape_seen.store(true, SeqCst); self.cancel(); }
    pub fn admit(&self, primary_down: bool, escape_down: bool) -> bool {
        if escape_down { self.escape(); }
        if !primary_down { self.cancel(); }
        !self.cancelled()
    }
    /// Cancel is durable and always wins over the initiating button's release.
    pub fn decide(&self, escape: bool, primary_down: bool) -> Decision {
        let mut count=self.queries.load(SeqCst);
        while count<10_000 {
            match self.queries.compare_exchange(count,count+1,SeqCst,SeqCst) {
                Ok(_)=>break, Err(current)=>count=current,
            }
        }
        if escape { self.escape(); }
        if self.cancelled() { Decision::Cancel } else if primary_down { Decision::Continue } else { Decision::Drop }
    }
}
#[derive(Debug, PartialEq)]
pub enum Decision { Continue, Cancel, Drop }

struct Page {
    window: String, page: String, last_sequence: u64, active: Option<Arc<Gesture>>,
}
#[derive(Default)]
pub struct Registry { pages: HashMap<isize, Page> }
pub fn token(s: &str) -> bool { s.len() == 32 && s.bytes().all(|b| b.is_ascii_hexdigit()) }
impl Registry {
    pub fn bind(&mut self, hwnd: isize, window: &str, page: &str) -> bool {
        if hwnd == 0 || window.is_empty() || !token(page) { return false; }
        if self.pages.get(&hwnd).is_some_and(|p| p.window == window && p.page == page) { return true; }
        self.remove(hwnd);
        self.pages.insert(hwnd, Page { window: window.into(), page: page.into(), last_sequence: 0, active: None });
        true
    }
    pub fn begin(&mut self, hwnd: isize, window: &str, page: &str, nonce: &str, sequence: u64) -> Option<Arc<Gesture>> {
        let p = self.pages.get_mut(&hwnd)?;
        if p.window != window || p.page != page || !token(nonce) || sequence == 0 || sequence > 9_007_199_254_740_991 || sequence <= p.last_sequence || p.active.is_some() { return None; }
        p.last_sequence = sequence;
        let g = Arc::new(Gesture { window: window.into(), page: page.into(), nonce: nonce.into(), sequence,
            cancelled: AtomicBool::new(false), started: AtomicBool::new(false), entered_ole: AtomicBool::new(false), focus_verified: AtomicBool::new(false), escape_seen: AtomicBool::new(false), queries: AtomicU32::new(0) });
        p.active = Some(g.clone()); Some(g)
    }
    fn find(&self, hwnd: isize, window: &str, page: &str, nonce: &str, sequence: u64) -> Option<Arc<Gesture>> {
        let p = self.pages.get(&hwnd)?;
        let g = p.active.as_ref()?;
        (p.window == window && p.page == page && g.window == window && g.page == page && g.nonce == nonce && g.sequence == sequence).then(|| g.clone())
    }
    pub fn cancel(&self, hwnd: isize, window: &str, page: &str, nonce: &str, sequence: u64) -> bool {
        let Some(g) = self.find(hwnd, window, page, nonce, sequence) else { return false };
        g.escape(); true
    }
    pub fn claim(&self, hwnd: isize, window: &str, page: &str, nonce: &str, sequence: u64) -> Option<Arc<Gesture>> {
        let g = self.find(hwnd, window, page, nonce, sequence)?;
        g.started.compare_exchange(false, true, SeqCst, SeqCst).ok()?; Some(g)
    }
    pub fn active_matches(&self, hwnd: isize, window: &str, page: &str, nonce: &str, sequence: u64) -> bool {
        self.find(hwnd,window,page,nonce,sequence).is_some()
    }
    pub fn cancel_window(&self, hwnd: isize) {
        if let Some(g) = self.pages.get(&hwnd).and_then(|p| p.active.as_ref()) { g.escape(); }
    }
    pub fn finish(&mut self, hwnd: isize, gesture: &Arc<Gesture>) {
        if let Some(p) = self.pages.get_mut(&hwnd) {
            if p.active.as_ref().is_some_and(|g| Arc::ptr_eq(g, gesture)) { p.active = None; }
        }
    }
    pub fn remove(&mut self, hwnd: isize) {
        if let Some(p) = self.pages.remove(&hwnd) { if let Some(g) = p.active { g.cancel(); } }
    }
}
pub fn registry() -> &'static Mutex<Registry> {
    static REGISTRY: OnceLock<Mutex<Registry>> = OnceLock::new(); REGISTRY.get_or_init(Default::default)
}

/// Focus restoration is permitted only while our source still owns foreground
/// and focus, and the saved child still belongs to that same source window.
pub fn restore_focus(source_foreground: bool, source_focused: bool, saved_owned: bool) -> bool {
    source_foreground && source_focused && saved_owned
}

#[cfg(test)]
mod tests {
    use super::*;
    const PAGE: &str = "11111111111111111111111111111111";
    const NONCE: &str = "22222222222222222222222222222222";
    fn setup() -> (Registry, Arc<Gesture>) {
        let mut r = Registry::default(); assert!(r.bind(1,"one",PAGE));
        let g = r.begin(1,"one",PAGE,NONCE,1).unwrap(); (r,g)
    }
    #[test] fn quick_escape_before_queue_is_durable_and_claimed_once() {
        let (r,g)=setup(); assert!(r.cancel(1,"one",PAGE,NONCE,1));
        assert!(r.claim(1,"one",PAGE,NONCE,1).is_some()); assert!(r.claim(1,"one",PAGE,NONCE,1).is_none());
        assert!(!g.admit(true,false)); assert_eq!(g.decide(false,false),Decision::Cancel);
    }
    #[test] fn cancel_after_begin_wins_over_drop() {
        let (r,g)=setup(); r.claim(1,"one",PAGE,NONCE,1).unwrap();
        assert!(g.admit(true,false)); assert_eq!(g.decide(false,true),Decision::Continue);
        r.cancel(1,"one",PAGE,NONCE,1); assert_eq!(g.decide(false,false),Decision::Cancel);
    }
    #[test] fn foreign_stale_and_replayed_requests_do_not_affect_new_gesture() {
        let (mut r,g)=setup();
        for (h,w,p,n,s) in [(2,"one",PAGE,NONCE,1),(1,"other",PAGE,NONCE,1),(1,"one",NONCE,NONCE,1),(1,"one",PAGE,PAGE,1),(1,"one",PAGE,NONCE,2)] { assert!(!r.cancel(h,w,p,n,s)); }
        assert!(!g.cancelled()); r.finish(1,&g);
        assert!(r.begin(1,"one",PAGE,NONCE,1).is_none());
        let next=r.begin(1,"one",PAGE,PAGE,2).unwrap(); r.finish(1,&g);
        assert!(r.claim(1,"one",PAGE,PAGE,2).is_some()); assert!(!next.cancelled());
    }
    #[test] fn release_before_preparation_or_before_ole_aborts() {
        let (_,a)=setup(); assert!(!a.admit(false,false));
        let (_,b)=setup(); assert!(b.admit(true,false)); assert!(!b.admit(false,false));
        assert_eq!(b.decide(false,false),Decision::Cancel);
    }
    #[test] fn positive_and_native_escape_policy() {
        let (_,g)=setup(); assert!(g.admit(true,false)); assert!(g.admit(true,false));
        assert_eq!(g.decide(false,true),Decision::Continue); assert_eq!(g.decide(false,false),Decision::Drop);
        let (_,g)=setup(); assert_eq!(g.decide(true,false),Decision::Cancel); assert_eq!(g.decide(false,false),Decision::Cancel);
    }
    #[test] fn reload_close_and_old_finish_retire_without_touching_replacement() {
        let (mut r,g)=setup(); assert!(r.bind(1,"one",NONCE)); assert!(g.cancelled());
        let next=r.begin(1,"one",NONCE,PAGE,1).unwrap(); r.finish(1,&g); assert!(!next.cancelled());
        r.remove(1); assert!(next.cancelled()); assert!(r.claim(1,"one",NONCE,PAGE,1).is_none());
    }
    #[test] fn restore_never_steals_foreign_focus() {
        assert!(!restore_focus(false,true,true),"Explorer became foreground");
        assert!(!restore_focus(true,false,true),"The user already focused another child");
        assert!(!restore_focus(true,true,false),"The saved child no longer belongs to the source");
        assert!(restore_focus(true,true,true));
    }
}
