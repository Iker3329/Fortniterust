//! Heap accounting of the browser build: a counting wrapper around std's allocator (dlmalloc) that tracks live and peak bytes, plus the size of the
//! wasm linear memory (it only grows: freed heap is reused by dlmalloc but never returned to the browser).
//!
//! Attribution (debug, `?fnweb_heaptrack=1`): every allocation of at least [`TRACK_MIN`] bytes records the wasm call stack of its site; the live ones
//! can be listed grouped by site ([`report`]). The tracker's own allocations bypass it (a per-thread guard), and it never holds its lock while
//! the allocator's lock is held. Smaller allocations are SAMPLED (one address in [`SAMPLE`], chosen by a hash of the address, so a free knows
//! whether its block was sampled without a lookup) and reported with that weight: most of a loaded heap is small objects.
use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Mutex;

pub struct Counting;

static LIVE: AtomicUsize = AtomicUsize::new(0);
static PEAK: AtomicUsize = AtomicUsize::new(0);
static ALLOCS: AtomicUsize = AtomicUsize::new(0);

pub const TRACK_MIN: usize = 64 << 10;
static TRACKING: AtomicBool = AtomicBool::new(false);
/// ptr -> (size x weight, site id); site id -> stack text
static TRACKED: Mutex<Option<(HashMap<usize, (usize, u64)>, HashMap<u64, String>)>> = Mutex::new(None);
/// Allocations below [`TRACK_MIN`]: one address in this many is tracked (weight SAMPLE).
const SAMPLE: usize = 64;
#[inline]
fn sampled(p: *mut u8) -> bool { TRACKING.load(Ordering::Relaxed) && ((p as usize as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15) >> 58) == 0 }
#[inline]
fn watched(p: *mut u8, size: usize) -> bool { size >= TRACK_MIN || sampled(p) }
#[inline]
fn weight(size: usize) -> usize { if size >= TRACK_MIN { size } else { size * SAMPLE } }

thread_local! { static IN_TRACKER: Cell<bool> = const { Cell::new(false) }; }

pub fn enable_tracking(on: bool) { TRACKING.store(on, Ordering::Relaxed); }

fn hash(s: &str) -> u64 { s.bytes().fold(0xcbf29ce484222325u64, |h, b| (h ^ b as u64).wrapping_mul(0x100000001b3)) }

fn track(p: *mut u8, size: usize) {
    if !TRACKING.load(Ordering::Relaxed) { return; }
    let _ = IN_TRACKER.try_with(|g| {
        if g.replace(true) { return; }
        let stack = crate::js::fnweb_stack();
        let id = hash(&stack);
        if let Ok(mut t) = TRACKED.lock() {
            let (live, sites) = t.get_or_insert_with(|| (HashMap::new(), HashMap::new()));
            live.insert(p as usize, (weight(size), id));
            sites.entry(id).or_insert(stack);
        }
        g.set(false);
    });
}

fn untrack(p: *mut u8) {
    if !TRACKING.load(Ordering::Relaxed) { return; }
    let _ = IN_TRACKER.try_with(|g| {
        if g.replace(true) { return; }
        if let Ok(mut t) = TRACKED.lock() { if let Some((live, _)) = t.as_mut() { live.remove(&(p as usize)); } }
        g.set(false);
    });
}

#[inline]
fn add(n: usize) {
    let live = LIVE.fetch_add(n, Ordering::Relaxed) + n;
    let mut p = PEAK.load(Ordering::Relaxed);
    while live > p {
        match PEAK.compare_exchange_weak(p, live, Ordering::Relaxed, Ordering::Relaxed) { Ok(_) => break, Err(x) => p = x }
    }
}

const CLASSES: [usize; 7] = [16, 32, 64, 128, 256, 512, 1024];
const CAPS: [u32; 7] = [64, 64, 64, 32, 16, 8, 8];
#[inline]
fn class_of(l: Layout) -> Option<usize> {
    if l.align() > 16 || l.size() == 0 || l.size() > 1024 { return None; }
    Some(CLASSES.iter().position(|&c| l.size() <= c).unwrap())
}
#[inline]
fn class_layout(c: usize) -> Layout { unsafe { Layout::from_size_align_unchecked(CLASSES[c], 16) } }
thread_local! {
    static FREE: [Cell<*mut u8>; 7] = const { [const { Cell::new(std::ptr::null_mut()) }; 7] };
    static COUNT: [Cell<u32>; 7] = const { [const { Cell::new(0) }; 7] };
}
#[inline]
unsafe fn small_alloc(c: usize) -> *mut u8 {
    let hit = FREE.try_with(|f| { let p = f[c].get(); if !p.is_null() { f[c].set(unsafe { *(p as *mut *mut u8) }); let _ = COUNT.try_with(|n| n[c].set(n[c].get() - 1)); } p }).unwrap_or(std::ptr::null_mut());
    if !hit.is_null() { return hit; }
    unsafe { System.alloc(class_layout(c)) }
}
#[inline]
unsafe fn small_free(p: *mut u8, c: usize) {
    let kept = COUNT.try_with(|n| { if n[c].get() >= CAPS[c] { return false; } FREE.try_with(|f| { unsafe { *(p as *mut *mut u8) = f[c].get(); } f[c].set(p); n[c].set(n[c].get() + 1); true }).unwrap_or(false) }).unwrap_or(false);
    if !kept { unsafe { System.dealloc(p, class_layout(c)) } }
}

unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, l: Layout) -> *mut u8 {
        let p = match class_of(l) { Some(c) => unsafe { small_alloc(c) }, None => unsafe { System.alloc(l) } };
        if !p.is_null() { add(l.size()); ALLOCS.fetch_add(1, Ordering::Relaxed); if watched(p, l.size()) { track(p, l.size()); } }
        p
    }
    unsafe fn alloc_zeroed(&self, l: Layout) -> *mut u8 {
        let p = match class_of(l) { Some(c) => { let p = unsafe { small_alloc(c) }; if !p.is_null() { unsafe { std::ptr::write_bytes(p, 0, l.size()) }; } p } None => unsafe { System.alloc_zeroed(l) } };
        if !p.is_null() { add(l.size()); ALLOCS.fetch_add(1, Ordering::Relaxed); if watched(p, l.size()) { track(p, l.size()); } }
        p
    }
    unsafe fn dealloc(&self, p: *mut u8, l: Layout) {
        if watched(p, l.size()) { untrack(p); }
        match class_of(l) { Some(c) => unsafe { small_free(p, c) }, None => unsafe { System.dealloc(p, l) } }
        LIVE.fetch_sub(l.size(), Ordering::Relaxed);
    }
    unsafe fn realloc(&self, p: *mut u8, l: Layout, new: usize) -> *mut u8 {
        let nl = unsafe { Layout::from_size_align_unchecked(new, l.align()) };
        let (oc, nc) = (class_of(l), class_of(nl));
        if oc.is_none() && nc.is_none() {
            if watched(p, l.size()) { untrack(p); }
            let q = unsafe { System.realloc(p, l, new) };
            if !q.is_null() {
                if new >= l.size() { add(new - l.size()); } else { LIVE.fetch_sub(l.size() - new, Ordering::Relaxed); }
                if watched(q, new) { track(q, new); }
            } else if watched(p, l.size()) { track(p, l.size()); }
            return q;
        }
        if oc.is_some() && oc == nc {
            if new >= l.size() { add(new - l.size()); } else { LIVE.fetch_sub(l.size() - new, Ordering::Relaxed); }
            return p;
        }
        let q = unsafe { self.alloc(nl) };
        if !q.is_null() { unsafe { std::ptr::copy_nonoverlapping(p, q, l.size().min(new)); self.dealloc(p, l); } }
        q
    }
}

#[global_allocator]
static GLOBAL: Counting = Counting;

/// (live heap bytes, peak live heap bytes, wasm linear memory bytes, allocations so far)
pub fn stats() -> (usize, usize, usize, usize) {
    (LIVE.load(Ordering::Relaxed), PEAK.load(Ordering::Relaxed), core::arch::wasm32::memory_size(0) * 65536, ALLOCS.load(Ordering::Relaxed))
}

/// Resets the peak to the current live size (e.g. after loading, to see the steady state).
pub fn reset_peak() { PEAK.store(LIVE.load(Ordering::Relaxed), Ordering::Relaxed); }

/// The live tracked allocations grouped by call site, biggest first: "MB  count  site".
pub fn report(top: usize) -> String {
    let rows: Vec<(usize, usize, String)> = {
        let _g = IN_TRACKER.try_with(|g| g.replace(true)).unwrap_or(true);
        let t = TRACKED.lock();
        let mut by: HashMap<u64, (usize, usize)> = HashMap::new();
        let mut out = vec![];
        if let Ok(t) = t {
            if let Some((live, sites)) = t.as_ref() {
                for (_, (size, id)) in live.iter() { let e = by.entry(*id).or_default(); e.0 += size; e.1 += 1; }
                for (id, (bytes, n)) in by { out.push((bytes, n, sites.get(&id).cloned().unwrap_or_default())); }
            }
        }
        let _ = IN_TRACKER.try_with(|g| g.set(false));
        out
    };
    let mut rows = rows;
    rows.sort_by(|a, b| b.0.cmp(&a.0));
    let total: usize = rows.iter().map(|r| r.0).sum();
    let mut s = format!("tracked live allocations (>= {} KiB exact, smaller ones sampled 1:64 and scaled): ~{:.0} MB in {} sites (heap live {:.0} MB)\n", TRACK_MIN >> 10, total as f64 / 1048576., rows.len(), LIVE.load(Ordering::Relaxed) as f64 / 1048576.);
    for (bytes, n, site) in rows.into_iter().take(top) { s.push_str(&format!("{:8.1} MB {:5}x  {site}\n", bytes as f64 / 1048576., n)); }
    s
}
