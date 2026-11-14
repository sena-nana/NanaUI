//! What the platform's fonts cost in memory once their bytes are needed.
//!
//! This reads whatever fonts the machine has installed, so it is `#[ignore]`d
//! like the rest of the platform acceptance (see
//! `font_system_platform_acceptance.rs`). Run it on the target platform:
//!
//! ```text
//! cargo test -p nana-text --test system_font_memory -- --ignored --nocapture
//! ```
//!
//! System font files are mapped, not read (`font/mapped.rs`): asking for
//! every installed face's bytes must leave them out of the heap. Read whole,
//! the same loop holds every font file on the machine, several gigabytes on
//! a stock macOS.

use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicI64, Ordering};

use nana_text::font::FontSystem;

struct Counting;

static LIVE: AtomicI64 = AtomicI64::new(0);

unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        LIVE.fetch_add(layout.size() as i64, Ordering::Relaxed);
        unsafe { System.alloc(layout) }
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        LIVE.fetch_add(layout.size() as i64, Ordering::Relaxed);
        unsafe { System.alloc_zeroed(layout) }
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        LIVE.fetch_add(new_size as i64 - layout.size() as i64, Ordering::Relaxed);
        unsafe { System.realloc(ptr, layout, new_size) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        LIVE.fetch_sub(layout.size() as i64, Ordering::Relaxed);
        unsafe { System.dealloc(ptr, layout) }
    }
}

#[global_allocator]
static GLOBAL: Counting = Counting;

#[test]
#[ignore = "reads the machine's installed fonts; run on the target platform"]
fn every_system_face_can_be_read_without_holding_its_file_on_the_heap() {
    let system = FontSystem::with_system_fonts();
    let faces = system.faces();
    assert!(!faces.is_empty(), "the platform scan found no fonts");
    let before = LIVE.load(Ordering::Relaxed);
    let mut held = Vec::new();
    let mut files: Vec<*const u8> = Vec::new();
    let mut file_bytes = 0usize;
    for face in &faces {
        let Some(data) = system.face_data(*face) else {
            continue;
        };
        let start = data.bytes().as_ptr();
        if !files.contains(&start) {
            files.push(start);
            file_bytes += data.bytes().len();
        }
        held.push(data);
    }
    let heap = (LIVE.load(Ordering::Relaxed) - before).max(0) as usize;
    println!(
        "{} faces in {} files, {:.1} MiB of font files, {:.2} MiB of heap",
        held.len(),
        files.len(),
        file_bytes as f64 / 1_048_576.0,
        heap as f64 / 1_048_576.0
    );
    assert!(
        heap < file_bytes / 64,
        "{heap} bytes of heap for {file_bytes} bytes of font files"
    );
}
