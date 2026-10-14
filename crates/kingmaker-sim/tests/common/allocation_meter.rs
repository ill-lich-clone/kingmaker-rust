//! Single-measurement allocation meter; tracks live bytes globally so freeing pre-existing
//! values during a measurement cannot underflow an artificially reset live counter.
use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering::Relaxed};
use std::sync::Mutex;
pub static MEASURE: Mutex<()> = Mutex::new(());
static TRACK: AtomicBool = AtomicBool::new(false);
static TOTAL: AtomicUsize = AtomicUsize::new(0);
static LIVE: AtomicUsize = AtomicUsize::new(0);
static PEAK: AtomicUsize = AtomicUsize::new(0);
pub struct Allocator;
fn added(n: usize) {
    let live = LIVE.fetch_add(n, Relaxed) + n;
    if TRACK.load(Relaxed) {
        TOTAL.fetch_add(n, Relaxed);
        PEAK.fetch_max(live, Relaxed);
    }
}
unsafe impl GlobalAlloc for Allocator {
    unsafe fn alloc(&self, l: Layout) -> *mut u8 {
        let p = System.alloc(l);
        if !p.is_null() { added(l.size()); }
        p
    }
    unsafe fn alloc_zeroed(&self, l: Layout) -> *mut u8 {
        let p = System.alloc_zeroed(l);
        if !p.is_null() { added(l.size()); }
        p
    }
    unsafe fn dealloc(&self, p: *mut u8, l: Layout) {
        LIVE.fetch_sub(l.size(), Relaxed);
        System.dealloc(p, l);
    }
    unsafe fn realloc(&self, p: *mut u8, l: Layout, n: usize) -> *mut u8 {
        let q = System.realloc(p, l, n);
        if !q.is_null() { LIVE.fetch_sub(l.size(), Relaxed); added(n); }
        q
    }
}
pub struct Measurement { baseline: usize }
impl Measurement {
    pub fn start() -> Self {
        let baseline = LIVE.load(Relaxed);
        TOTAL.store(0, Relaxed);
        PEAK.store(baseline, Relaxed);
        TRACK.store(true, Relaxed);
        Self { baseline }
    }
    pub fn finish(self) -> (usize, usize) {
        TRACK.store(false, Relaxed);
        (TOTAL.load(Relaxed), PEAK.load(Relaxed).saturating_sub(self.baseline))
    }
}
impl Drop for Measurement {
    fn drop(&mut self) { TRACK.store(false, Relaxed); }
}
