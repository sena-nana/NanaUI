//! The least thread stack a layout needs at a tree depth, and what one takes.
//!
//! ```text
//! cargo run --release -- search --engine nana-frame --depth 500
//! cargo run --release -- report            # every engine at depths 64..500
//! cargo run --release -- wide              # a wide page: time, heap, allocations per box
//! cargo run --release -- wide-loop         # that page over and over, for a profiler
//! /usr/bin/time -l target/release/nana-stack-depth reps 3200 10 nana
//!                                          # a fixed run: instructions and cycles per box;
//!                                          # SKIP_LAYOUT=1 builds the page only
//! BUILD_ALLOCS=1 cargo run --release -- wide # also the build's allocations and heap per box
//! FOOTPRINT=1 target/release/nana-stack-depth reps 1000 1
//!                                          # the page's live heap beside what the process
//!                                          # grew by in physical memory, per box
//! HOLD=1 target/release/nana-stack-depth reps 1000 1
//!                                          # keep the page alive for `malloc_history`
//! FRAME_PATH=1 cargo run --release -- wide  # the product frame's first layout
//!                                          # (`layout_document_for_viewport`), not a full pass
//! CONTROLS=1 cargo run --release -- wide    # a page of labels and buttons instead of bare boxes
//! CONTROLS=1 MIXED=1 cargo run --release -- edits 2000 height
//!                                          # closed selects among the buttons
//! CONTROLS=1 MIXED=1 OPEN_SELECT=1 cargo run --release -- edits 2000 height
//!                                          # the edit in the second row, and an open select
//!                                          # in the fourth: in view and moved by every edit
//!                                          # (`OPEN_SELECT=closed`: the same page, the select closed)
//! VISIBLE=1 cargo run --release -- edits    # cull the scene after each frame, as a painter does
//! SCROLLED=1 cargo run --release -- edits 2000 height
//!                                          # the rows in a scroller scrolled before the edits
//! ROTATED=1 cargo run --release -- edits 2000 height
//!                                          # the rows in a column turned by 0.05 rad
//! ROTATED=1 cargo run --release -- edits 2000 transform
//!                                          # turn the rows' column by another angle each edit
//!                                          # (`scale`: scale it); a plain page turns its root
//! cargo run --release -- edits 200,2000   # one edit a product frame: layout and frame time
//! RUNS=240 /usr/bin/time -l target/release/nana-stack-depth edits 2000 height
//!                                          # 240 edits; less RUNS=40, over 200: one edit's instructions
//! cargo run --release -- stages 250,500    # a deep frame's least time per stage
//! cargo run --release -- insert 100,200,400 deep,wide,under-deep
//!                                          # mount into a laid-out page: mount and next frame
//! ```
//!
//! A probe runs the work once on a thread of the stack it is given; a stack
//! overflow aborts the process, so `search` runs each probe as a child
//! process and bisects. `nana-layout` and `taffy` lay out the same nested
//! flex columns (a quarter pixel of padding each); `nana-frame` takes a whole
//! NanaUI frame through the same tree: mount, layout, scene extraction, hit
//! testing, accessibility, an update and teardown.

use std::alloc::{GlobalAlloc, Layout, System};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicIsize, Ordering};
use std::time::Instant;

use nana_ui_core::{LayoutStyle, LengthSpec};
use nana_ui_runtime::view::{IntoView, text, widget};
use nana_ui_runtime::{
    AppContext, Button, ComputedStyle, DocumentId, FrameStage, LayoutViewport, MutationQueue,
    StableNodeId, Stack, Text, TextContent, TextMetrics, TextShapeConstraints, TextShaper,
};
use nana_ui_scene::RuntimeDocument;

thread_local! {
    /// The scroller a `SCROLLED=1` page holds its rows in.
    static SCROLLER: std::cell::Cell<Option<StableNodeId>> = const { std::cell::Cell::new(None) };
    /// The column holding the rows, which the `transform` and `scale` edits
    /// turn or scale: the turned column on a `ROTATED=1` page, the page's
    /// root otherwise.
    static ROWS: std::cell::Cell<Option<StableNodeId>> = const { std::cell::Cell::new(None) };
}

const ENGINES: [&str; 3] = ["nana-layout", "nana-frame", "taffy"];

/// Live heap bytes: allocations minus frees.
static LIVE: AtomicIsize = AtomicIsize::new(0);
/// Allocation calls (`alloc`, `alloc_zeroed`, `realloc`).
static CALLS: AtomicIsize = AtomicIsize::new(0);

struct Counting;

unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        CALLS.fetch_add(1, Ordering::Relaxed);
        LIVE.fetch_add(layout.size() as isize, Ordering::Relaxed);
        unsafe { System.alloc(layout) }
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        CALLS.fetch_add(1, Ordering::Relaxed);
        LIVE.fetch_add(layout.size() as isize, Ordering::Relaxed);
        unsafe { System.alloc_zeroed(layout) }
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        CALLS.fetch_add(1, Ordering::Relaxed);
        LIVE.fetch_add(
            new_size as isize - layout.size() as isize,
            Ordering::Relaxed,
        );
        unsafe { System.realloc(ptr, layout, new_size) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        LIVE.fetch_sub(layout.size() as isize, Ordering::Relaxed);
        unsafe { System.dealloc(ptr, layout) }
    }
}

#[global_allocator]
static GLOBAL: Counting = Counting;

/// The wide page: a 1280 px column of `rows` rows of `cells` cells. Returns
/// the middle row's sixth cell's leaf (its label on a controls page), which
/// `edits` changes.
fn nana_page(cx: &mut AppContext, document: DocumentId, rows: usize, cells: usize) -> StableNodeId {
    // `BURN=n`: n nodes created and removed first, the ids a session that
    // has been running a while has behind it.
    if let Some(burn) = std::env::var("BURN")
        .ok()
        .and_then(|burn| burn.parse::<usize>().ok())
    {
        for _ in 0..burn {
            let node = cx
                .create_detached_component(document, Stack::row(0.0))
                .unwrap();
            cx.remove_view(node).unwrap();
        }
    }
    let mut target = None;
    // `SCROLLED=1`: the rows stand in a column inside a scroller a window
    // tall, which `edits` scrolls before it edits.
    let scrolled = std::env::var_os("SCROLLED").is_some();
    let page = Stack::column(0.0).width(LengthSpec::Px(1280.0));
    let page = if scrolled {
        page.with_layout(|layout| {
            layout.height = Some(LengthSpec::Px(800.0));
            layout.overflow_y = nana_ui_core::OverflowSpec::Auto;
        })
    } else {
        page
    };
    let mut root = cx.create_component(document, page).unwrap();
    // `ROTATED=1`: the rows stand in a column turned by 0.05 rad, so every
    // box under it hangs from a rotation.
    let rotated = std::env::var_os("ROTATED").is_some();
    if scrolled || rotated {
        let list = Stack::column(0.0).width(LengthSpec::Px(1280.0));
        let list = if rotated {
            let (sin, cos) = 0.05_f32.sin_cos();
            list.with_layout(|layout| {
                layout.transforms.transform = Some(nana_ui_core::PaintTransform {
                    a: cos,
                    b: sin,
                    c: -sin,
                    d: cos,
                    e: 0.0,
                    f: 0.0,
                });
            })
        } else {
            list
        };
        let list = cx.create_detached_component(document, list).unwrap();
        cx.append_child(root, list).unwrap();
        if scrolled {
            SCROLLER.with(|scroller| scroller.set(Some(root.stable_id())));
        }
        root = list;
    }
    ROWS.with(|rows| rows.set(Some(root.stable_id())));
    // `CONTROLS=1`: a page of controls, not bare boxes -- every cell holds a
    // label and a button, so the passes meet visuals, text and their tables.
    let controls = std::env::var_os("CONTROLS").is_some();
    // `MIXED=1` with it: every odd cell's button is a closed select, a
    // visual whose geometry once read the page.
    let mixed = controls && std::env::var_os("MIXED").is_some();
    // `OPEN_SELECT=1` with it: the edit is in the second row, and the
    // fourth row's second cell is an open select, in view and moved by
    // every edit.
    let open_select = mixed && std::env::var_os("OPEN_SELECT").is_some();
    let target_row = if open_select { 1 } else { rows / 2 };
    for r in 0..rows {
        let row = cx
            .create_detached_component(document, Stack::row(4.0))
            .unwrap();
        cx.append_child(root, row).unwrap();
        for c in 0..cells {
            if controls {
                let cell = cx
                    .create_detached_component(document, Stack::column(2.0))
                    .unwrap();
                cx.append_child(row, cell).unwrap();
                let label = cx
                    .create_detached_component(document, Text::new("Label"))
                    .unwrap();
                cx.append_child(cell, label).unwrap();
                if (r, c) == (target_row, cells / 2) {
                    target = Some(label.stable_id());
                }
                if mixed && c % 2 == 1 {
                    let mut select = nana_ui_runtime::Select::new(Some("a"));
                    select.options = vec![
                        nana_ui_runtime::SelectOption::new("a", "Alpha"),
                        nana_ui_runtime::SelectOption::new("b", "Beta"),
                    ];
                    if open_select
                        && (r, c) == (3, 1)
                        && std::env::var_os("OPEN_SELECT").is_some_and(|value| value != "closed")
                    {
                        select.opened = true;
                        select.highlighted = Some(0);
                    }
                    let select = cx.create_detached_component(document, select).unwrap();
                    cx.append_child(cell, select).unwrap();
                    continue;
                }
                let button = cx
                    .create_detached_component(document, Button::new("Go"))
                    .unwrap();
                cx.append_child(cell, button).unwrap();
                continue;
            }
            let cell = cx
                .create_detached_component(
                    document,
                    Stack::column(0.0).with_layout(|l| *l = padded()),
                )
                .unwrap();
            cx.append_child(row, cell).unwrap();
            let leaf = cx
                .create_detached_component(
                    document,
                    Stack::row(0.0)
                        .width(LengthSpec::Px(24.0))
                        .min_height(LengthSpec::Px(16.0)),
                )
                .unwrap();
            cx.append_child(cell, leaf).unwrap();
            if (r, c) == (rows / 2, cells / 2) {
                target = Some(leaf.stable_id());
            }
        }
    }
    target.unwrap()
}

/// A page-shaped tree: `rows` rows of `cells` cells under one column, each
/// cell a column holding a row (four levels, `1 + rows * (1 + 2 * cells)`
/// boxes). Returns the microseconds the layout took and those the build and
/// layout took together, the live heap bytes
/// the laid-out tree holds and the allocation calls the layout made, the
/// first and last without building it.
fn nana_wide(rows: usize, cells: usize) -> (f64, f64, isize, isize) {
    let document = DocumentId::new(1).unwrap();
    let before = LIVE.load(Ordering::Relaxed);
    let footprint = physical_footprint();
    let built = Instant::now();
    let build_calls = CALLS.load(Ordering::Relaxed);
    let mut cx = AppContext::new();
    nana_page(&mut cx, document, rows, cells);
    let calls = CALLS.load(Ordering::Relaxed);
    // `BUILD_ALLOCS=1`: the allocation calls and live heap the build made,
    // per box, before any layout.
    if std::env::var_os("BUILD_ALLOCS").is_some() {
        let boxes = (1 + rows * (1 + 2 * cells)) as f64;
        eprintln!(
            "build: {:.1} allocations / box, {:.0} B / box held",
            (calls - build_calls) as f64 / boxes,
            (LIVE.load(Ordering::Relaxed) - before) as f64 / boxes
        );
    }
    let start = Instant::now();
    // `SKIP_LAYOUT` leaves the build alone in a `reps` run: subtracting it
    // gives the layout's own instructions and cycles.
    if std::env::var_os("SKIP_LAYOUT").is_some() {
    } else if std::env::var_os("FRAME_PATH").is_some() {
        // The product frame's first layout: a viewport pass over a document
        // laid out for the first time.
        cx.layout_document_for_viewport(document, LayoutViewport::new(1280.0, 800.0), &[])
            .unwrap();
    } else {
        cx.layout_document(document, LayoutViewport::new(1280.0, 800.0))
            .unwrap();
    }
    let took = start.elapsed().as_secs_f64() * 1e6;
    let total = built.elapsed().as_secs_f64() * 1e6;
    let calls = CALLS.load(Ordering::Relaxed) - calls;
    let held = LIVE.load(Ordering::Relaxed) - before;
    // `FOOTPRINT=1`: what the process grew by in physical memory, beside the
    // live heap. The two differ by what the allocator keeps of blocks the
    // page freed on the way (a table that grew by copying leaves its earlier
    // copies there) less the capacity nothing has written to yet.
    report_footprint(rows, cells, held, footprint);
    // `HOLD=1`: keep the laid-out page alive, for a heap tool
    // (`MallocStackLogging=1` and `malloc_history <pid> -allBySize`).
    if std::env::var_os("HOLD").is_some() {
        eprintln!("holding {held} B");
        std::thread::sleep(std::time::Duration::from_secs(3600));
    }
    drop(cx);
    (took, total, held, calls)
}

fn report_footprint(rows: usize, cells: usize, held: isize, footprint_before: isize) {
    if std::env::var_os("FOOTPRINT").is_some() {
        let boxes = (1 + rows * (1 + 2 * cells)) as f64;
        eprintln!(
            "held: {:.0} B / box live, {:.0} B / box physical",
            held as f64 / boxes,
            (physical_footprint() - footprint_before) as f64 / boxes
        );
    }
}

/// The physical memory the process is charged with (`phys_footprint`, what
/// Activity Monitor shows), in bytes; zero where it is not read.
fn physical_footprint() -> isize {
    #[cfg(target_os = "macos")]
    {
        unsafe extern "C" {
            fn proc_pid_rusage(pid: i32, flavor: i32, buffer: *mut u64) -> i32;
        }
        // `rusage_info_v0`: a 16-byte uuid, then 64-bit fields; the footprint
        // is the eighth of them.
        let mut info = [0u64; 16];
        if unsafe { proc_pid_rusage(std::process::id() as i32, 0, info.as_mut_ptr()) } == 0 {
            return info[9] as isize;
        }
    }
    0
}

/// The same page laid out by Taffy.
/// [`nana_page`] in Taffy: the tree, its root and the leaf `edits` changes.
fn taffy_page(rows: usize, cells: usize) -> (taffy::TaffyTree<()>, taffy::NodeId, taffy::NodeId) {
    use taffy::prelude::*;
    let mut tree: TaffyTree<()> = TaffyTree::new();
    let mut target = None;
    let padding = Rect {
        left: length(0.25_f32),
        right: length(0.25_f32),
        top: length(0.25_f32),
        bottom: length(0.25_f32),
    };
    let mut row_nodes = Vec::with_capacity(rows);
    for r in 0..rows {
        let mut cell_nodes = Vec::with_capacity(cells);
        for c in 0..cells {
            let leaf = tree
                .new_leaf(Style {
                    size: Size {
                        width: length(24.0_f32),
                        height: auto(),
                    },
                    min_size: Size {
                        width: auto(),
                        height: length(16.0_f32),
                    },
                    ..Default::default()
                })
                .unwrap();
            if (r, c) == (rows / 2, cells / 2) {
                target = Some(leaf);
            }
            let cell = tree
                .new_with_children(
                    Style {
                        flex_direction: FlexDirection::Column,
                        padding,
                        ..Default::default()
                    },
                    &[leaf],
                )
                .unwrap();
            cell_nodes.push(cell);
        }
        let row = tree
            .new_with_children(
                Style {
                    gap: Size {
                        width: length(4.0_f32),
                        height: length(4.0_f32),
                    },
                    ..Default::default()
                },
                &cell_nodes,
            )
            .unwrap();
        row_nodes.push(row);
    }
    let root = tree
        .new_with_children(
            Style {
                flex_direction: FlexDirection::Column,
                size: Size {
                    width: length(1280.0_f32),
                    height: auto(),
                },
                ..Default::default()
            },
            &row_nodes,
        )
        .unwrap();
    (tree, root, target.unwrap())
}

fn taffy_wide(rows: usize, cells: usize) -> (f64, f64, isize, isize) {
    use taffy::prelude::*;
    let before = LIVE.load(Ordering::Relaxed);
    let footprint = physical_footprint();
    let built = Instant::now();
    let (mut tree, root, _) = taffy_page(rows, cells);
    let calls = CALLS.load(Ordering::Relaxed);
    let start = Instant::now();
    tree.compute_layout(
        root,
        Size {
            width: AvailableSpace::Definite(1280.0),
            height: AvailableSpace::Definite(800.0),
        },
    )
    .unwrap();
    let took = start.elapsed().as_secs_f64() * 1e6;
    let total = built.elapsed().as_secs_f64() * 1e6;
    let calls = CALLS.load(Ordering::Relaxed) - calls;
    let held = LIVE.load(Ordering::Relaxed) - before;
    report_footprint(rows, cells, held, footprint);
    drop(tree);
    (took, total, held, calls)
}

/// One edit to the [`nana_page`] at a time, each in a product frame
/// (`RuntimeDocument::flush`) after the first: the least layout stage and
/// the least whole flush over `runs` edits, in microseconds. `width` and
/// `height` toggle the target leaf's width or minimum height (the second
/// moves every row below it); `text` toggles a controls page's label;
/// `transform` turns the rows' column (by 0.05 or 0.06 rad on a `ROTATED=1`
/// page, 0.01 or 0.02 otherwise) and `scale` scales it (1.01 or 1.02).
fn nana_edits(rows: usize, cells: usize, edit: &str, runs: usize) -> (f64, f64) {
    let document = DocumentId::new(1).unwrap();
    let mut runtime = RuntimeDocument::new(document);
    let target = nana_page(runtime.context_mut(), document, rows, cells);
    let viewport = LayoutViewport::new(1280.0, 800.0);
    // `VISIBLE=1`: cull the scene after every frame, as a painter does, so
    // the scene keeps its visibility index and pays for keeping it current.
    let visible = std::env::var_os("VISIBLE").is_some();
    let cull = |runtime: &RuntimeDocument| {
        if visible {
            let rect = nana_ui_scene::SceneRect {
                x: 0.0,
                y: 0.0,
                width: 1280.0,
                height: 800.0,
            };
            std::hint::black_box(runtime.scene().visible_operations(rect).unwrap());
        }
    };
    runtime.flush(viewport, &mut FixedShaper).unwrap();
    if let Some(scroller) = SCROLLER.with(|scroller| scroller.get()) {
        let mut queue = MutationQueue::new();
        queue.set_scroll_offset(
            scroller,
            nana_ui_runtime::ScrollOffset {
                x: 0.0,
                y: 12_345.5,
            },
        );
        runtime.context_mut().commit_mutations(queue).unwrap();
        runtime.flush(viewport, &mut FixedShaper).unwrap();
    }
    cull(&runtime);
    let (mut layout, mut whole) = (f64::INFINITY, f64::INFINITY);
    // The least time each stage took over the runs, by stage.
    let mut least: Vec<(FrameStage, f64)> = Vec::new();
    for run in 0..runs {
        let on = run % 2 == 0;
        let cx = runtime.context_mut();
        let mut queue = MutationQueue::new();
        if edit == "text" {
            let value = if on { "Label two" } else { "Label" };
            queue.set_text(
                target,
                TextContent {
                    value: value.into(),
                },
            );
        } else if edit == "transform" || edit == "scale" {
            let rows = ROWS.with(|rows| rows.get()).unwrap();
            let mut style = cx.world().node_style(rows).unwrap().clone();
            let turn = if std::env::var_os("ROTATED").is_some() {
                0.05
            } else {
                0.01
            };
            // The first edit changes it: the page starts at `turn`.
            let step = if on { 0.01 } else { 0.0 };
            let (sin, cos) = (turn + step as f32).sin_cos();
            let scale = 1.01 + step as f32;
            std::sync::Arc::make_mut(&mut style.layout).transforms.transform = Some(if edit == "transform" {
                nana_ui_core::PaintTransform {
                    a: cos,
                    b: sin,
                    c: -sin,
                    d: cos,
                    e: 0.0,
                    f: 0.0,
                }
            } else {
                nana_ui_core::PaintTransform {
                    a: scale,
                    b: 0.0,
                    c: 0.0,
                    d: scale,
                    e: 0.0,
                    f: 0.0,
                }
            });
            queue.set_style(rows, style);
        } else {
            let mut style = cx.world().node_style(target).unwrap().clone();
            let spec = std::sync::Arc::make_mut(&mut style.layout);
            match edit {
                "width" => spec.width = Some(LengthSpec::Px(if on { 32.0 } else { 24.0 })),
                "height" => spec.min_height = Some(LengthSpec::Px(if on { 24.0 } else { 16.0 })),
                other => panic!("unknown edit {other}"),
            }
            queue.set_style(target, style);
        }
        cx.commit_mutations(queue).unwrap();
        let start = Instant::now();
        runtime.flush(viewport, &mut FixedShaper).unwrap();
        whole = whole.min(start.elapsed().as_secs_f64() * 1e6);
        cull(&runtime);
        let stage: f64 = runtime
            .context()
            .last_frame_profile()
            .stages
            .iter()
            .filter(|timing| timing.stage == FrameStage::Layout)
            .map(|timing| timing.duration.as_secs_f64() * 1e6)
            .sum();
        layout = layout.min(stage);
        for timing in &runtime.context().last_frame_profile().stages {
            let micros = timing.duration.as_secs_f64() * 1e6;
            match least.iter_mut().find(|(stage, _)| *stage == timing.stage) {
                Some((_, held)) => *held = held.min(micros),
                None => least.push((timing.stage, micros)),
            }
        }
        if run + 1 == runs && std::env::var_os("PRINT_STAGES").is_some() {
            let stages: Vec<String> = least
                .iter()
                .filter(|(_, micros)| *micros > 0.0)
                .map(|(stage, micros)| format!("{stage:?} {micros:.0}"))
                .collect();
            eprintln!("{edit}: least {} (µs)", stages.join(", "));
        }
    }
    (layout, whole)
}

/// `insert`: lay a wide page of `rows` rows out, then mount `size` more
/// nodes into it in one `mount_view` under the target leaf (`shape`
/// `deep`: a chain `size` columns deep; `wide`: `size` sibling columns as
/// the view's roots; `under-deep`: a chain 400 deep first, then `size`
/// siblings at its bottom, beside its text; `CHAIN` sets its depth;
/// `under-deep-at-once`: that tree mounted in one go). Returns the least
/// mount (build and insert commits) and the least frame after it, in
/// microseconds. `PRINT_STAGES=1` adds the frame's least time per stage,
/// `PRINT_COUNTERS=1` its work counters.
fn nana_insert(rows: usize, size: usize, shape: &str, runs: usize) -> (f64, f64) {
    let (mut mount, mut frame) = (f64::INFINITY, f64::INFINITY);
    // The least time each stage of the frame after the mount took, by stage.
    let mut least: Vec<(FrameStage, f64)> = Vec::new();
    for run in 0..runs {
        let document = DocumentId::new(1).unwrap();
        let mut runtime = RuntimeDocument::new(document);
        let mut target = nana_page(runtime.context_mut(), document, rows, 10);
        let viewport = LayoutViewport::new(1280.0, 800.0);
        runtime.flush(viewport, &mut FixedShaper).unwrap();
        let chain = |depth: usize| {
            let mut view = text("leaf").into_any();
            for _ in 0..depth {
                view = widget(Stack::column(0.0).with_layout(|l| *l = padded()))
                    .children((view,))
                    .into_any();
            }
            view
        };
        // `CHAIN=<depth>`: the depth of `under-deep`'s chain (400).
        let depth = std::env::var("CHAIN").map_or(400, |depth| depth.parse().unwrap());
        let column = || widget(Stack::column(0.0).with_layout(|l| *l = padded()));
        if shape == "under-deep" {
            let made = runtime
                .context_mut()
                .mount_view(target, move || chain(depth))
                .unwrap();
            runtime.flush(viewport, &mut FixedShaper).unwrap();
            let mut bottom = made.roots()[0];
            while let Some(child) = runtime
                .context()
                .world()
                .node(bottom)
                .and_then(|node| node.children.first().copied())
            {
                if runtime
                    .context()
                    .world()
                    .node(child)
                    .unwrap()
                    .children
                    .is_empty()
                {
                    break;
                }
                bottom = child;
            }
            target = bottom;
        }
        let start = Instant::now();
        match shape {
            "deep" => {
                runtime
                    .context_mut()
                    .mount_view(target, move || chain(size))
                    .unwrap();
            }
            "under-deep-at-once" => {
                // `under-deep`'s tree mounted in one go: what its frame
                // after compares with.
                runtime
                    .context_mut()
                    .mount_view(target, move || {
                        let bottom = std::iter::once(text("leaf").into_any())
                            .chain((0..size).map(|_| column().into_any()))
                            .collect::<Vec<_>>();
                        let mut view = column().children(bottom).into_any();
                        for _ in 1..depth {
                            view = column().children((view,)).into_any();
                        }
                        view
                    })
                    .unwrap();
            }
            "wide" | "under-deep" => {
                runtime
                    .context_mut()
                    .mount_view(target, move || {
                        (0..size).map(|_| column().into_any()).collect::<Vec<_>>()
                    })
                    .unwrap();
            }
            other => panic!("unknown shape {other}"),
        }
        mount = mount.min(start.elapsed().as_secs_f64() * 1e6);
        let start = Instant::now();
        runtime.flush(viewport, &mut FixedShaper).unwrap();
        frame = frame.min(start.elapsed().as_secs_f64() * 1e6);
        for timing in &runtime.context().last_frame_profile().stages {
            let micros = timing.duration.as_secs_f64() * 1e6;
            match least.iter_mut().find(|(stage, _)| *stage == timing.stage) {
                Some((_, held)) => *held = held.min(micros),
                None => least.push((timing.stage, micros)),
            }
        }
    }
    if std::env::var_os("PRINT_STAGES").is_some() {
        let stages: Vec<String> = least
            .iter()
            .filter(|(_, micros)| *micros > 0.0)
            .map(|(stage, micros)| format!("{stage:?} {micros:.0}"))
            .collect();
        eprintln!("{shape} {size}: least {} (µs)", stages.join(", "));
    }
    (mount, frame)
}

/// [`nana_edits`] in Taffy: the least `compute_layout` after each edit.
fn taffy_edits(rows: usize, cells: usize, edit: &str, runs: usize) -> f64 {
    use taffy::prelude::*;
    let (mut tree, root, target) = taffy_page(rows, cells);
    let available = Size {
        width: AvailableSpace::Definite(1280.0),
        height: AvailableSpace::Definite(800.0),
    };
    tree.compute_layout(root, available).unwrap();
    let mut least = f64::INFINITY;
    for run in 0..runs {
        let on = run % 2 == 0;
        let mut style = tree.style(target).unwrap().clone();
        match edit {
            "width" => style.size.width = length(if on { 32.0_f32 } else { 24.0 }),
            "height" => style.min_size.height = length(if on { 24.0_f32 } else { 16.0 }),
            other => panic!("unknown edit {other}"),
        }
        tree.set_style(target, style).unwrap();
        let start = Instant::now();
        tree.compute_layout(root, available).unwrap();
        least = least.min(start.elapsed().as_secs_f64() * 1e6);
    }
    least
}

struct FixedShaper;

impl TextShaper for FixedShaper {
    fn shape(
        &mut self,
        _id: StableNodeId,
        text: &TextContent,
        _style: &ComputedStyle,
        _constraints: TextShapeConstraints,
    ) -> TextMetrics {
        TextMetrics {
            width: text.value.len() as f32 * 8.0,
            height: 18.0,
            ascent: None,
        }
    }
}

fn padded() -> LayoutStyle {
    LayoutStyle {
        padding: Some(LengthSpec::Px(0.25)),
        ..Default::default()
    }
}

/// `depth` nested columns laid out once by NanaUI's layout engine; returns
/// the microseconds the layout took, without building the tree.
fn nana_layout(depth: usize) -> f64 {
    let document = DocumentId::new(1).unwrap();
    let mut cx = AppContext::new();
    let mut parent = cx
        .create_component(document, Stack::column(0.0).width(LengthSpec::Px(800.0)))
        .unwrap();
    for _ in 0..depth {
        let child = cx
            .create_detached_component(document, Stack::column(0.0).with_layout(|l| *l = padded()))
            .unwrap();
        cx.append_child(parent, child).unwrap();
        parent = child;
    }
    let start = Instant::now();
    cx.layout_document(document, LayoutViewport::new(800.0, 600.0))
        .unwrap();
    start.elapsed().as_secs_f64() * 1e6
}

/// A whole frame through `depth` nested columns; returns the microseconds
/// from mounting to the frame's scene, hit index and accessibility tree.
fn nana_frame(depth: usize) -> f64 {
    let start = Instant::now();
    let document = DocumentId::new(1).unwrap();
    let mut runtime = RuntimeDocument::new(document);
    runtime
        .context_mut()
        .mount_view_root(document, move || {
            let mut view = text("leaf").into_any();
            for _ in 0..depth {
                view = widget(Stack::column(0.0).with_layout(|l| *l = padded()))
                    .children((view,))
                    .into_any();
            }
            view
        })
        .unwrap();
    let viewport = LayoutViewport::new(800.0, 600.0);
    runtime.flush(viewport, &mut FixedShaper).unwrap();
    let cx = runtime.context();
    let _ = cx.world().hit_test(document, 1.0, 1.0);
    let _ = cx.world().project_accessibility(document);
    let took = start.elapsed().as_secs_f64() * 1e6;
    drop(runtime);
    took
}

/// One frame through `depth` nested columns, per stage: the least each named
/// stage took over `runs` frames, in microseconds.
fn nana_frame_stages(depth: usize, runs: usize) -> Vec<(String, f64)> {
    let mut least: Vec<(String, f64)> = Vec::new();
    for _ in 0..runs {
        let document = DocumentId::new(1).unwrap();
        let mut runtime = RuntimeDocument::new(document);
        let mount = Instant::now();
        runtime
            .context_mut()
            .mount_view_root(document, move || {
                let mut view = text("leaf").into_any();
                for _ in 0..depth {
                    view = widget(Stack::column(0.0).with_layout(|l| *l = padded()))
                        .children((view,))
                        .into_any();
                }
                view
            })
            .unwrap();
        let mounted = ("Mount".to_owned(), mount.elapsed());
        runtime
            .flush(LayoutViewport::new(800.0, 600.0), &mut FixedShaper)
            .unwrap();
        let profile = runtime.context().last_frame_profile().clone();
        let stages = std::iter::once(mounted).chain(
            profile
                .stages
                .into_iter()
                .map(|timing| (format!("{:?}", timing.stage), timing.duration)),
        );
        for (name, duration) in stages {
            let us = duration.as_secs_f64() * 1e6;
            match least.iter_mut().find(|(n, _)| *n == name) {
                Some((_, held)) => *held = held.min(us),
                None => least.push((name, us)),
            }
        }
    }
    least.retain(|(_, us)| *us > 0.0);
    least
}

/// The same nested columns laid out once by Taffy; returns the microseconds
/// the layout took, without building the tree.
fn taffy(depth: usize) -> f64 {
    use taffy::prelude::*;
    let mut tree: TaffyTree<()> = TaffyTree::new();
    let padding = Rect {
        left: length(0.25_f32),
        right: length(0.25_f32),
        top: length(0.25_f32),
        bottom: length(0.25_f32),
    };
    let column = Style {
        flex_direction: FlexDirection::Column,
        padding,
        ..Default::default()
    };
    let mut node = tree.new_leaf(column.clone()).unwrap();
    for _ in 0..depth {
        node = tree.new_with_children(column.clone(), &[node]).unwrap();
    }
    let root = tree
        .new_with_children(
            Style {
                flex_direction: FlexDirection::Column,
                size: Size {
                    width: length(800.0_f32),
                    height: auto(),
                },
                ..Default::default()
            },
            &[node],
        )
        .unwrap();
    let start = Instant::now();
    tree.compute_layout(
        root,
        Size {
            width: AvailableSpace::Definite(800.0),
            height: AvailableSpace::Definite(600.0),
        },
    )
    .unwrap();
    start.elapsed().as_secs_f64() * 1e6
}

fn run(engine: &str, depth: usize) -> f64 {
    match engine {
        "nana-layout" => nana_layout(depth),
        "nana-frame" => nana_frame(depth),
        "taffy" => taffy(depth),
        other => panic!("unknown engine {other}"),
    }
}

/// Runs `engine` once on a thread of `stack` bytes.
fn probe(engine: String, depth: usize, stack: usize) {
    std::thread::Builder::new()
        .stack_size(stack)
        .spawn(move || {
            run(&engine, depth);
        })
        .unwrap()
        .join()
        .unwrap();
}

/// Whether a probe of `stack` bytes finishes, in a child process.
fn fits(engine: &str, depth: usize, stack: usize) -> bool {
    Command::new(std::env::current_exe().unwrap())
        .args(["probe", engine, &depth.to_string(), &stack.to_string()])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|status| status.success())
}

/// The least stack, to 4 KiB, a probe finishes on.
fn least_stack(engine: &str, depth: usize) -> Option<usize> {
    const PAGE: usize = 4 << 10;
    let (mut low, mut high) = (PAGE, 256 << 20);
    if !fits(engine, depth, high) {
        return None;
    }
    while high - low > PAGE {
        let mid = (low + high) / 2 / PAGE * PAGE;
        if fits(engine, depth, mid) {
            high = mid;
        } else {
            low = mid;
        }
    }
    Some(high)
}

/// Fastest of `rounds` runs on an ample stack, in microseconds.
fn fastest(engine: &str, depth: usize, rounds: usize) -> f64 {
    let engine = engine.to_owned();
    std::thread::Builder::new()
        .stack_size(256 << 20)
        .spawn(move || {
            (0..rounds)
                .map(|_| run(&engine, depth))
                .fold(f64::INFINITY, f64::min)
        })
        .unwrap()
        .join()
        .unwrap()
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let arg = |i: usize| args.get(i).map(String::as_str);
    match arg(0) {
        Some("probe") => probe(
            args[1].clone(),
            args[2].parse().unwrap(),
            args[3].parse().unwrap(),
        ),
        Some("search") => {
            let engine = args
                .iter()
                .position(|a| a == "--engine")
                .map_or("nana-layout", |i| args[i + 1].as_str());
            let depth = args
                .iter()
                .position(|a| a == "--depth")
                .map_or(500, |i| args[i + 1].parse().unwrap());
            match least_stack(engine, depth) {
                Some(bytes) => println!("{engine} depth {depth}: {} KiB", bytes >> 10),
                None => println!("{engine} depth {depth}: fails at 256 MiB"),
            }
        }
        Some("report") | None => {
            let build = if cfg!(debug_assertions) {
                "debug"
            } else {
                "release"
            };
            println!("| engine ({build}) | depth | least stack | fastest run |");
            println!("| --- | ---: | ---: | ---: |");
            let depths: Vec<usize> = match arg(1) {
                Some(list) => list.split(',').map(|d| d.parse().unwrap()).collect(),
                None => vec![64, 128, 256, 500],
            };
            for depth in depths {
                for engine in ENGINES {
                    let stack = (arg(2) != Some("time"))
                        .then(|| least_stack(engine, depth))
                        .flatten()
                        .map_or("-".to_owned(), |b| format!("{} KiB", b >> 10));
                    let time = fastest(engine, depth, 20);
                    println!("| {engine} | {depth} | {stack} | {time:.0} µs |");
                }
            }
        }
        Some("wide") => {
            // A page of rows and cells: layout time and held heap per box.
            let build = if cfg!(debug_assertions) {
                "debug"
            } else {
                "release"
            };
            println!(
                "| engine ({build}) | boxes | fastest layout | per box | build + layout per box | heap per box | allocations per box |"
            );
            println!("| --- | ---: | ---: | ---: | ---: | ---: | ---: |");
            // `wide 50,200,800` picks the row counts (10 cells a row).
            let rows: Vec<usize> = match arg(1) {
                Some(list) => list.split(',').map(|r| r.parse().unwrap()).collect(),
                None => vec![100, 500, 1000],
            };
            for (rows, cells) in rows.into_iter().map(|rows| (rows, 10)) {
                // A controls page's cell holds a label and a button.
                let per_cell = if std::env::var_os("CONTROLS").is_some() {
                    3
                } else {
                    2
                };
                let boxes = 1 + rows * (1 + per_cell * cells);
                for (name, run) in [
                    (
                        "nana-layout",
                        nana_wide as fn(usize, usize) -> (f64, f64, isize, isize),
                    ),
                    ("taffy", taffy_wide),
                ] {
                    // Taffy lays out the bare-box page only.
                    if name == "taffy" && std::env::var_os("CONTROLS").is_some() {
                        continue;
                    }
                    let (mut fastest, mut whole) = (f64::INFINITY, f64::INFINITY);
                    let (mut held, mut calls) = (0, 0);
                    for _ in 0..15 {
                        let (took, total, bytes, made) = run(rows, cells);
                        fastest = fastest.min(took);
                        whole = whole.min(total);
                        (held, calls) = (bytes, made);
                    }
                    println!(
                        "| {name} | {boxes} | {fastest:.0} µs | {:.2} µs | {:.2} µs | {} B | {:.1} |",
                        fastest / boxes as f64,
                        whole / boxes as f64,
                        held / boxes as isize,
                        calls as f64 / boxes as f64
                    );
                }
            }
        }
        Some("edits") => {
            // One edit a frame on the wide page: `edits 200,2000 [width,height]`.
            // A controls page (`CONTROLS=1`) edits its label's text.
            let rows: Vec<usize> = match arg(1) {
                Some(list) => list.split(',').map(|r| r.parse().unwrap()).collect(),
                None => vec![200, 2000, 6400],
            };
            let controls = std::env::var_os("CONTROLS").is_some();
            let edits: Vec<&str> = match arg(2) {
                Some(list) => list.split(',').collect(),
                None if controls => vec!["text"],
                None => vec!["width", "height"],
            };
            println!("| engine | boxes | edit | layout | whole frame | taffy layout |");
            println!("| --- | ---: | --- | ---: | ---: | ---: |");
            // `RUNS=n`: edits a page (40). Two counts' `time -l` difference
            // over their edits is what one edit costs, the page and its
            // first frame taken out.
            let runs = std::env::var("RUNS").map_or(40, |runs| runs.parse().unwrap());
            for rows in rows {
                let boxes = 1 + rows * (1 + if controls { 3 } else { 2 } * 10);
                for edit in &edits {
                    let (layout, whole) = nana_edits(rows, 10, edit, runs);
                    // Taffy lays out; it has no paint transform to turn.
                    let taffy = if controls || matches!(*edit, "transform" | "scale") {
                        "-".to_owned()
                    } else {
                        format!("{:.1} µs", taffy_edits(rows, 10, edit, 40))
                    };
                    println!(
                        "| nana | {boxes} | {edit} | {layout:.1} µs | {whole:.1} µs | {taffy} |"
                    );
                }
            }
        }
        Some("insert") => {
            // Mount into a laid-out page: `insert 100,200,400 deep|wide|under-deep`.
            let sizes: Vec<usize> = match arg(1) {
                Some(list) => list.split(',').map(|n| n.parse().unwrap()).collect(),
                None => vec![100, 200, 400],
            };
            let shapes: Vec<&str> = match arg(2) {
                Some(list) => list.split(',').collect(),
                None => vec!["deep", "wide", "under-deep"],
            };
            let rows = std::env::var("ROWS").map_or(200, |rows| rows.parse().unwrap());
            let runs = std::env::var("RUNS").map_or(10, |runs| runs.parse().unwrap());
            println!("| shape | nodes | mount | frame after |");
            println!("| --- | ---: | ---: | ---: |");
            for shape in shapes {
                for &size in &sizes {
                    let (mount, frame) = nana_insert(rows, size, shape, runs);
                    println!("| {shape} | {size} | {mount:.0} µs | {frame:.0} µs |");
                }
            }
        }
        Some("reps") => {
            // A fixed amount of work for `time -l`: `reps <rows> <runs> [taffy]`.
            let rows: usize = arg(1).unwrap().parse().unwrap();
            let runs: usize = arg(2).unwrap().parse().unwrap();
            let taffy = arg(3) == Some("taffy");
            for _ in 0..runs {
                if taffy {
                    taffy_wide(rows, 10);
                } else {
                    nana_wide(rows, 10);
                }
            }
        }
        Some("wide-loop") => {
            // For a profiler: lay the 10 501-box page out over and over.
            let rows = arg(1).map_or(500, |rows| rows.parse().unwrap());
            loop {
                nana_wide(rows, 10);
            }
        }
        Some("stages") => {
            // Where a deep frame's time goes: the least per stage over runs.
            // The runtime refuses a tree deeper than 512.
            let depths: Vec<usize> = match arg(1) {
                Some(list) => list.split(',').map(|d| d.parse().unwrap()).collect(),
                None => vec![125, 250, 500],
            };
            for depth in depths {
                let stages = nana_frame_stages(depth, 15);
                let line: Vec<String> = stages
                    .iter()
                    .map(|(name, us)| format!("{name} {us:.0}"))
                    .collect();
                println!("depth {depth}: {} (µs)", line.join(", "));
            }
        }
        Some("loop") => {
            // For a profiler: run one engine over and over.
            let depth = args[2].parse().unwrap();
            loop {
                fastest(&args[1], depth, 1);
            }
        }
        Some(other) => panic!("unknown command {other}"),
    }
}
