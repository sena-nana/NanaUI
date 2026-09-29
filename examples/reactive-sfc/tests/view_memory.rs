//! Live heap of compiled views against their hand-written twins:
//! allocations minus frees, counted per thread so parallel tests do not
//! disturb each other.

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;

use nana_ui::runtime::view::{IntoView, Signal, column, signal};
use nana_ui::runtime::{AppContext, DocumentId};
use reactive_sfc::bench::{idiomatic, views};

struct Counting;

thread_local! {
    static LIVE: Cell<i64> = const { Cell::new(0) };
}

fn add(bytes: i64) {
    let _ = LIVE.try_with(|live| live.set(live.get() + bytes));
}

unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        add(layout.size() as i64);
        unsafe { System.alloc(layout) }
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        add(layout.size() as i64);
        unsafe { System.alloc_zeroed(layout) }
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        add(new_size as i64 - layout.size() as i64);
        unsafe { System.realloc(ptr, layout, new_size) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        add(-(layout.size() as i64));
        unsafe { System.dealloc(ptr, layout) }
    }
}

#[global_allocator]
static GLOBAL: Counting = Counting;

/// Live bytes per row after mounting `n` rows from `row`.
fn live_per_row<V: IntoView>(n: usize, row: fn(usize, Signal<usize>) -> V) -> f64 {
    let document = DocumentId::new(1).unwrap();
    let mut cx = AppContext::typed();
    let mount = |cx: &mut AppContext, n: usize| {
        cx.mount_view_root(document, move || {
            let selected = signal(0);
            column().children((0..n).map(|i| row(i, selected)).collect::<Vec<_>>())
        })
        .unwrap()
    };
    // Grow the thread's signal runtime, the context's tables and the
    // process-wide defaults first.
    drop(mount(&mut AppContext::typed(), n));
    let warm = mount(&mut cx, 8);
    let before = LIVE.with(Cell::get);
    let rows = mount(&mut cx, n);
    let per_row = (LIVE.with(Cell::get) - before) as f64 / n as f64;
    drop((rows, warm, cx));
    per_row
}

/// A row with a conditional class keeps its base layout to recompose
/// from. Every instance shares one: holding its own would cost a whole
/// `LayoutStyle` (about 2 KB) per row over the same row written by hand.
#[test]
fn conditional_classes_share_their_base_layout() {
    const N: usize = 1_000;
    let compiled = live_per_row(N, views::styled_row);
    let by_hand = live_per_row(N, idiomatic::styled_row);
    assert!(
        compiled < by_hand + 512.0,
        "{compiled:.0} B per compiled row, {by_hand:.0} B by hand"
    );
}
