use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;

thread_local! {
    pub static COUNTING: Cell<bool> = const { Cell::new(false) };
    pub static ALLOCATIONS: Cell<u64> = const { Cell::new(0) };
    pub static TOTAL_ALLOCATIONS: Cell<u64> = const { Cell::new(0) };
    pub static BYTES: Cell<u64> = const { Cell::new(0) };
    pub static ALLOCATED_GROWTH: Cell<u64> = const { Cell::new(0) };
    pub static LIVE: Cell<i64> = const { Cell::new(0) };
}

struct CountingAllocator;

fn record_alloc(pointer: *mut u8, size: usize) {
    if !pointer.is_null() && COUNTING.try_with(Cell::get).unwrap_or(false) {
        let _ = ALLOCATIONS.try_with(|c| c.set(c.get() + 1));
    }
    let _ = TOTAL_ALLOCATIONS.try_with(|c| c.set(c.get() + 1));
    let _ = BYTES.try_with(|b| b.set(b.get() + size as u64));
}

// SAFETY: every call forwards pointer and layout unchanged to System;
// accounting touches only allocation-free thread-local cells.
unsafe impl GlobalAlloc for CountingAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let size = layout.size();
        let _ = LIVE.try_with(|l| l.set(l.get() + size as i64));
        let _ = ALLOCATED_GROWTH.try_with(|a| a.set(a.get() + size as u64));
        let pointer = unsafe { System.alloc(layout) };
        record_alloc(pointer, size);
        pointer
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        let size = layout.size();
        let _ = LIVE.try_with(|l| l.set(l.get() + size as i64));
        let _ = ALLOCATED_GROWTH.try_with(|a| a.set(a.get() + size as u64));
        let pointer = unsafe { System.alloc_zeroed(layout) };
        record_alloc(pointer, size);
        pointer
    }

    unsafe fn dealloc(&self, pointer: *mut u8, layout: Layout) {
        let _ = LIVE.try_with(|l| l.set(l.get() - layout.size() as i64));
        unsafe { System.dealloc(pointer, layout) };
    }

    unsafe fn realloc(&self, pointer: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        let _ = LIVE.try_with(|l| l.set(l.get() + new_size as i64 - layout.size() as i64));
        let _ = ALLOCATED_GROWTH
            .try_with(|a| a.set(a.get() + new_size.saturating_sub(layout.size()) as u64));
        let pointer = unsafe { System.realloc(pointer, layout, new_size) };
        record_alloc(pointer, new_size);
        pointer
    }
}

#[global_allocator]
static GLOBAL: CountingAllocator = CountingAllocator;

mod i18n_alloc;
mod input_alloc;
mod metric_delta;
mod node_memory;
mod reactive_view_alloc;
mod smooth_scroll_cost;
mod tree_size_cost;
#[cfg(feature = "view-macro")]
mod view_stack;
