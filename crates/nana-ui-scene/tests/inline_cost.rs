//! Fixed-machine before/after fixture for #188/#191. The same executable can
//! be preserved before a migration and interleaved with the new executable.
#![cfg(feature = "benchmark")]

use nana_text::{
    NativeTextEngine,
    font::{FaceDescriptor, FallbackPolicy, FontSystem, GenericFamily},
};
use nana_ui_runtime::{
    Button, Card, DocumentId, LabeledValue, LayoutViewport, ListItem, MutationQueue,
    NanaTextEngineShaper, Stack, Text,
};
use nana_ui_scene::RuntimeDocument;
use serde::Serialize;
use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicUsize, Ordering},
};
use std::time::Instant;

struct Counted;
static LIVE: AtomicUsize = AtomicUsize::new(0);
static ALLOCS: AtomicUsize = AtomicUsize::new(0);
// SAFETY: the system allocator receives every original pointer/layout. The
// atomics neither allocate nor change allocation alignment or ownership.
unsafe impl GlobalAlloc for Counted {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let p = unsafe { System.alloc(layout) };
        if !p.is_null() {
            LIVE.fetch_add(layout.size(), Ordering::Relaxed);
            ALLOCS.fetch_add(1, Ordering::Relaxed);
        }
        p
    }
    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        let p = unsafe { System.alloc_zeroed(layout) };
        if !p.is_null() {
            LIVE.fetch_add(layout.size(), Ordering::Relaxed);
            ALLOCS.fetch_add(1, Ordering::Relaxed);
        }
        p
    }
    unsafe fn dealloc(&self, p: *mut u8, layout: Layout) {
        LIVE.fetch_sub(layout.size(), Ordering::Relaxed);
        unsafe { System.dealloc(p, layout) }
    }
    unsafe fn realloc(&self, p: *mut u8, layout: Layout, size: usize) -> *mut u8 {
        let next = unsafe { System.realloc(p, layout, size) };
        if !next.is_null() {
            LIVE.fetch_sub(layout.size(), Ordering::Relaxed);
            LIVE.fetch_add(size, Ordering::Relaxed);
            ALLOCS.fetch_add(1, Ordering::Relaxed);
        }
        next
    }
}
#[global_allocator]
static ALLOCATOR: Counted = Counted;

#[derive(Serialize)]
struct Case {
    kind: &'static str,
    nodes: usize,
    entities: usize,
    retained_live_bytes: usize,
    rss_kib: Option<u64>,
    idle_frame_p50_ns: f64,
    idle_frame_p95_ns: f64,
    idle_frame_p99_ns: f64,
    idle_allocations: usize,
}

fn rss() -> Option<u64> {
    let output = std::process::Command::new("ps")
        .args(["-o", "rss=", "-p", &std::process::id().to_string()])
        .output()
        .ok()?;
    std::str::from_utf8(&output.stdout)
        .ok()?
        .trim()
        .parse()
        .ok()
}

#[test]
fn inline_plain_text_fixed_machine_cost() {
    fixed_machine_cost(&["text", "button"]);
}

#[test]
#[ignore = "fixed-machine row cost; run separately to isolate the global allocator"]
fn list_item_fixed_machine_cost() {
    fixed_machine_cost(&["list_item"]);
}

#[test]
#[ignore = "fixed-machine value cost; run separately to isolate the global allocator"]
fn labeled_value_fixed_machine_cost() {
    fixed_machine_cost(&["labeled_value"]);
}

#[test]
#[ignore = "fixed-machine card cost; run separately to isolate the global allocator"]
fn card_fixed_machine_cost() {
    fixed_machine_cost(&["card"]);
}

fn fixed_machine_cost(kinds: &[&'static str]) {
    let mut cases = Vec::new();
    for &kind in kinds {
        for count in [1_000, 10_000] {
            let before = LIVE.load(Ordering::Relaxed);
            let document = DocumentId::new(1).unwrap();
            let mut doc = RuntimeDocument::new(document);
            let mut policy = FallbackPolicy::empty();
            policy.set_generic(GenericFamily::SansSerif, ["Noto Sans SC"]);
            let mut fonts = FontSystem::with_policy(policy);
            fonts
                .register_file(
                    concat!(
                        env!("CARGO_MANIFEST_DIR"),
                        "/../nana-ui/assets/fonts/NotoSansSC-Variable.ttf"
                    ),
                    &FaceDescriptor::default(),
                )
                .unwrap();
            let mut shaper =
                NanaTextEngineShaper::new(Arc::new(Mutex::new(NativeTextEngine::new(fonts))));
            let cx = doc.context_mut();
            let root_value = if matches!(kind, "labeled_value" | "card") {
                Stack::column(0.0).width(nana_ui_runtime::LengthSpec::Px(800.0))
            } else {
                Stack::column(0.0)
            };
            let root = cx.create_component(document, root_value).unwrap();
            let mut q = MutationQueue::new();
            for _ in 0..count {
                let id = if kind == "text" {
                    cx.create_detached_component(document, Text::new("NanaUI label"))
                        .unwrap()
                        .stable_id()
                } else if kind == "labeled_value" {
                    cx.create_detached_component(document, LabeledValue::new("Revision", "42"))
                        .unwrap()
                        .stable_id()
                } else if kind == "card" {
                    cx.create_detached_component(
                        document,
                        Card::new().title("Activity").loading(true),
                    )
                    .unwrap()
                    .stable_id()
                } else if kind == "list_item" {
                    cx.create_detached_component(
                        document,
                        ListItem::new("NanaUI label").note("note").detail("detail"),
                    )
                    .unwrap()
                    .stable_id()
                } else {
                    cx.create_detached_component(document, Button::new("NanaUI label"))
                        .unwrap()
                        .stable_id()
                };
                q.insert(root.stable_id(), id, None);
            }
            cx.commit_mutations(q).unwrap();
            let viewport = LayoutViewport::new(800.0, 600.0);
            doc.flush(viewport, &mut shaper).unwrap();
            for _ in 0..32 {
                assert!(doc.flush(viewport, &mut shaper).unwrap().is_idle());
            }
            let retained = LIVE.load(Ordering::Relaxed).saturating_sub(before);
            let resident = rss();
            let mut samples = Vec::with_capacity(1_000);
            let allocs = ALLOCS.load(Ordering::Relaxed);
            for _ in 0..1_000 {
                let start = Instant::now();
                for _ in 0..64 {
                    assert!(doc.flush(viewport, &mut shaper).unwrap().is_idle());
                }
                samples.push(start.elapsed().as_nanos() as f64 / 64.0);
            }
            let idle_allocations = ALLOCS.load(Ordering::Relaxed) - allocs;
            samples.sort_by(f64::total_cmp);
            // Record existing host/frame bookkeeping too. The before/after gate
            // forbids adding allocations; it must not invent a zero baseline.
            assert_eq!(doc.context().world().len(), count + 1);
            cases.push(Case {
                kind,
                nodes: count,
                entities: count + 1,
                retained_live_bytes: retained,
                rss_kib: resident,
                idle_frame_p50_ns: samples[500],
                idle_frame_p95_ns: samples[949],
                idle_frame_p99_ns: samples[989],
                idle_allocations,
            });
        }
    }
    let report=serde_json::to_string_pretty(&serde_json::json!({"schema_version":1,"profile":if cfg!(debug_assertions){"debug"}else{"release"},"cases":cases})).unwrap();
    if let Some(path) = std::env::var_os("NANA_INLINE_COST_OUTPUT") {
        std::fs::write(path, report).unwrap();
    } else {
        println!("{report}");
    }
}
