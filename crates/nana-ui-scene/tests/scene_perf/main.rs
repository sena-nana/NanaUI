use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;
use std::sync::atomic::{AtomicBool, AtomicIsize, AtomicUsize, Ordering};
use std::sync::{Mutex, MutexGuard};

pub static LIVE: AtomicIsize = AtomicIsize::new(0);
pub static PEAK: AtomicIsize = AtomicIsize::new(0);
pub static ALLOCS: AtomicUsize = AtomicUsize::new(0);
pub static TRACE: AtomicBool = AtomicBool::new(false);
thread_local! { static GUARD: Cell<bool> = const { Cell::new(false) }; }
type AllocRecord = (usize, usize, std::sync::Arc<std::backtrace::Backtrace>);

pub static BIG: Mutex<Vec<AllocRecord>> = Mutex::new(Vec::new());
pub static SNAP: Mutex<(isize, Vec<AllocRecord>)> = Mutex::new((0, Vec::new()));

struct Counted;

pub fn bt(b: &std::backtrace::Backtrace) -> String {
    let s = b.to_string();
    let mut out = String::new();
    let mut n = 0;
    for line in s.lines() {
        let t = line.trim();
        if t.starts_with("at ") {
            if t.contains("nana-ui-runtime/src") {
                out.push_str("   ");
                out.push_str(t);
                out.push('\n');
            }
            continue;
        }
        if true {
            out.push_str(t);
            out.push('\n');
            n += 1;
            if n >= 60 {
                break;
            }
        }
    }
    out
}

fn track(ptr: *mut u8, old: Option<*mut u8>, size: usize, live: isize, new_peak: bool) {
    if !TRACE.load(Ordering::Relaxed) {
        return;
    }
    GUARD.with(|g| {
        if g.get() {
            return;
        }
        g.set(true);
        {
            let mut big = BIG.lock().unwrap();
            if let Some(old) = old {
                big.retain(|e| e.0 != old as usize);
            }
            if size >= 16384 {
                big.push((
                    ptr as usize,
                    size,
                    std::sync::Arc::new(std::backtrace::Backtrace::force_capture()),
                ));
            }
            if new_peak {
                let mut snap = SNAP.lock().unwrap();
                snap.0 = live;
                snap.1 = big.iter().map(|e| (e.0, e.1, e.2.clone())).collect();
            }
        }
        g.set(false);
    });
}
fn untrack(ptr: *mut u8) {
    if !TRACE.load(Ordering::Relaxed) {
        return;
    }
    GUARD.with(|g| {
        if g.get() {
            return;
        }
        g.set(true);
        BIG.lock().unwrap().retain(|e| e.0 != ptr as usize);
        g.set(false);
    });
}

fn grew(by: isize) -> (isize, bool) {
    let live = LIVE.fetch_add(by, Ordering::Relaxed) + by;
    let prev = PEAK.fetch_max(live, Ordering::Relaxed);
    (live, live > prev)
}

unsafe impl GlobalAlloc for Counted {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let (live, np) = grew(layout.size() as isize);
        ALLOCS.fetch_add(1, Ordering::Relaxed);
        let p = unsafe { System.alloc(layout) };
        if layout.size() >= 16384 || np {
            track(p, None, layout.size(), live, np && layout.size() >= 1024);
        }
        p
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        let (live, np) = grew(layout.size() as isize);
        ALLOCS.fetch_add(1, Ordering::Relaxed);
        let p = unsafe { System.alloc_zeroed(layout) };
        if layout.size() >= 16384 || np {
            track(p, None, layout.size(), live, np && layout.size() >= 1024);
        }
        p
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        LIVE.fetch_sub(layout.size() as isize, Ordering::Relaxed);
        if layout.size() >= 16384 {
            untrack(ptr);
        }
        unsafe { System.dealloc(ptr, layout) }
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, size: usize) -> *mut u8 {
        let (live, np) = grew(size as isize - layout.size() as isize);
        ALLOCS.fetch_add(1, Ordering::Relaxed);
        let p = unsafe { System.realloc(ptr, layout, size) };
        if size >= 16384 || layout.size() >= 16384 {
            track(p, Some(ptr), size, live, np);
        }
        p
    }
}

#[global_allocator]
static ALLOCATOR: Counted = Counted;

pub fn serial_lock() -> MutexGuard<'static, ()> {
    static LOCK: Mutex<()> = Mutex::new(());
    LOCK.lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

mod first_layout_working_memory;
#[cfg(feature = "benchmark")]
mod inline_cost;
mod session_churn;
