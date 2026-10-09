//! Issue #270, Gate F: a steady update of one localized message allocates a
//! bounded amount, the same whatever the size of the catalog and of the
//! document.
//!
//! The counting allocator counts per thread. Each case warms up first (the
//! message compiled, its formatters built, the output buffer grown), then
//! counts the commits of a steady run of argument updates of one counter.
//! What a commit may allocate is the resolved string the node shows, and
//! the bookkeeping of a node that changed: none of it grows with the other
//! nodes or the other messages.

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;
use std::sync::Arc;

use nana_ui_runtime::{
    AppContext, DocumentId, Locale, LocalizedText, MessageTable, MutationQueue, NodeKind,
    StableNodeId,
};

thread_local! {
    static COUNTING: Cell<bool> = const { Cell::new(false) };
    static ALLOCATIONS: Cell<u64> = const { Cell::new(0) };
}

struct CountingAllocator;

fn record(pointer: *mut u8) {
    if !pointer.is_null() && COUNTING.try_with(Cell::get).unwrap_or(false) {
        let _ = ALLOCATIONS.try_with(|count| count.set(count.get() + 1));
    }
}

// SAFETY: every call forwards the caller's pointer and layout unchanged to
// System; accounting touches only allocation-free thread-local cells.
unsafe impl GlobalAlloc for CountingAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let pointer = unsafe { System.alloc(layout) };
        record(pointer);
        pointer
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        let pointer = unsafe { System.alloc_zeroed(layout) };
        record(pointer);
        pointer
    }

    unsafe fn dealloc(&self, pointer: *mut u8, layout: Layout) {
        unsafe { System.dealloc(pointer, layout) };
    }

    unsafe fn realloc(&self, pointer: *mut u8, layout: Layout, size: usize) -> *mut u8 {
        let pointer = unsafe { System.realloc(pointer, layout, size) };
        record(pointer);
        pointer
    }
}

#[global_allocator]
static GLOBAL: CountingAllocator = CountingAllocator;

fn id(value: u64) -> StableNodeId {
    StableNodeId::new(value).unwrap()
}

const COUNTER: u64 = 1;

/// A document of `localized` localized nodes over a catalog of `messages`
/// messages, plus the counter.
fn document(localized: u64, messages: u64) -> AppContext {
    let mut table = MessageTable::new().with(
        "en-us",
        "files",
        "{count, plural, one {# file} other {# files}}",
    );
    for message in 0..messages {
        table = table.with("en-us", &format!("item.{message}"), "Item {count}");
    }
    let mut context = AppContext::new();
    context.set_message_catalog(Some(Arc::new(table)));
    context.set_default_locale(Locale::parse("en-us"));
    let document = DocumentId::new(1).unwrap();
    let mut queue = MutationQueue::new();
    queue.create(id(1_000_000), document, NodeKind::Document);
    queue.create(id(COUNTER), document, NodeKind::Text);
    queue.insert(id(1_000_000), id(COUNTER), None);
    queue.set_localized_text(
        id(COUNTER),
        Some(LocalizedText::new("files").arg("count", 1u32)),
    );
    for node in 0..localized {
        let text = id(2 + node);
        queue.create(text, document, NodeKind::Text);
        queue.insert(id(1_000_000), text, None);
        queue.set_localized_text(
            text,
            Some(LocalizedText::new(&format!("item.{}", node % messages)).arg("count", node)),
        );
    }
    context.commit_mutations(queue).unwrap();
    context
}

/// Allocations per commit of a steady run of counter updates.
fn steady_allocations(context: &mut AppContext) -> u64 {
    let updates: Vec<MutationQueue> = (0..256u32)
        .map(|count| {
            let mut queue = MutationQueue::new();
            queue.set_localized_text(
                id(COUNTER),
                Some(LocalizedText::new("files").arg("count", 1_000 + count)),
            );
            queue
        })
        .collect();
    let mut updates = updates.into_iter();
    // Warm up: the message compiled, the formatters built, buffers grown.
    for queue in updates.by_ref().take(16) {
        context.commit_mutations(queue).unwrap();
    }
    let mut counted = 0;
    let mut commits = 0;
    for queue in updates {
        ALLOCATIONS.with(|count| count.set(0));
        COUNTING.with(|counting| counting.set(true));
        context.commit_mutations(queue).unwrap();
        COUNTING.with(|counting| counting.set(false));
        counted += ALLOCATIONS.with(Cell::get);
        commits += 1;
    }
    counted / commits
}

#[test]
fn a_steady_message_update_allocates_the_same_at_any_size() {
    let small = steady_allocations(&mut document(100, 10));
    let large = steady_allocations(&mut document(10_000, 5_000));
    assert_eq!(
        small, large,
        "allocations grew with the document or catalog"
    );
    assert!(small <= 8, "{small} allocations per update");
}
