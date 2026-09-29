//! Per-event input routing cost on the L3 Runtime path, with no paint.
//!
//! Every case drives [`RuntimeAgentSession::pointer_event`], which is one
//! canonical event routed into the document and nothing else: no flush, no
//! layout, no scene. The number is what the input path itself costs, which the
//! hover benchmark cannot show because its L3 side flushes after every move.
//!
//! Cases:
//! - `move-same`: moves inside one row, so hover never changes.
//! - `move-rows`: moves one row per event down a 2000-row column.
//! - `move-scrolled`: the same sweep inside a scroll viewport, where hit
//!   testing walks candidates instead of the flat index.
//! - `drag-range`: moves while a range thumb holds pointer capture.
//!
//! Allocations are counted on the benchmark thread only, per event.
//!
//! ```bash
//! cargo run --release -p nana-ui-devtools --features runtime-agent \
//!   --bin nana-input-benchmark -- --rounds 15 --events 4000
//! ```
//! Compare two builds by alternating their runs and reading the minimum p50;
//! one run's p50 on a loaded machine is not a result.

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;
use std::process::ExitCode;
use std::sync::Arc;
use std::time::Instant;

use nana_ui::runtime::view::{IntoView, widget};
use nana_ui::runtime::{
    DocumentId, LengthSpec, NodeStyle, RangeField, RuntimeDocument, ScrollAxes, ScrollView, Stack,
    Text,
};
use nana_ui_devtools::agent::RuntimeAgentSession;
use nana_ui_platform::PointerPhase;

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

// SAFETY: every operation forwards the caller's pointer and layout unchanged
// to System; accounting touches only allocation-free thread-local cells.
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

const ROWS: usize = 2000;
const ROW_HEIGHT: f32 = 24.0;
const WIDTH: u32 = 800;
const HEIGHT: u32 = 600;

struct Args {
    rounds: usize,
    events: usize,
}

fn main() -> ExitCode {
    let args = match parse(std::env::args().skip(1)) {
        Ok(args) => args,
        Err(error) => {
            eprintln!("{error}");
            return ExitCode::FAILURE;
        }
    };
    for case in ["move-same", "move-rows", "move-scrolled", "drag-range"] {
        match run(case, &args) {
            Ok(line) => println!("{line}"),
            Err(error) => {
                eprintln!("{case}: {error}");
                return ExitCode::FAILURE;
            }
        }
    }
    ExitCode::SUCCESS
}

fn run(case: &str, args: &Args) -> Result<String, String> {
    let mut session = match case {
        "move-same" | "move-rows" => session(rows_document(false))?,
        "move-scrolled" => session(rows_document(true))?,
        "drag-range" => session(range_document())?,
        _ => unreachable!("known case"),
    };
    if case == "drag-range" {
        session
            .pointer_event(PointerPhase::Down, 100.0, 20.0, 0, 1)
            .map_err(|error| error.to_string())?;
    }
    let mut per_event_ns = Vec::with_capacity(args.rounds);
    let mut allocations = 0_u64;
    let mut routed = 0_u64;
    for _ in 0..args.rounds {
        ALLOCATIONS.with(|count| count.set(0));
        COUNTING.with(|counting| counting.set(true));
        let started = Instant::now();
        for event in 0..args.events {
            let (x, y) = position(case, event);
            let buttons = u16::from(case == "drag-range");
            session
                .pointer_event(PointerPhase::Move, x, y, -1, buttons)
                .map_err(|error| error.to_string())?;
        }
        let elapsed = started.elapsed();
        COUNTING.with(|counting| counting.set(false));
        allocations += ALLOCATIONS.with(Cell::get);
        routed += args.events as u64;
        per_event_ns.push(elapsed.as_nanos() as f64 / args.events as f64);
    }
    per_event_ns.sort_by(f64::total_cmp);
    let p50 = per_event_ns[per_event_ns.len() / 2];
    let min = per_event_ns[0];
    Ok(format!(
        "{{\"case\":\"{case}\",\"events\":{routed},\"ns_per_event_p50\":{p50:.1},\"ns_per_event_min\":{min:.1},\"allocations_per_event\":{:.3}}}",
        allocations as f64 / routed as f64
    ))
}

fn position(case: &str, event: usize) -> (f32, f32) {
    match case {
        // Within one row, never crossing its edge.
        "move-same" => (40.0 + (event % 200) as f32, 4.0 + (event % 16) as f32),
        // One row per event down the visible part of the column.
        "move-rows" | "move-scrolled" => {
            let visible = (HEIGHT as f32 / ROW_HEIGHT) as usize;
            (
                60.0,
                (event % visible) as f32 * ROW_HEIGHT + ROW_HEIGHT / 2.0,
            )
        }
        "drag-range" => (40.0 + (event % 300) as f32, 20.0),
        _ => unreachable!("known case"),
    }
}

fn session(document: RuntimeDocument) -> Result<RuntimeAgentSession, String> {
    RuntimeAgentSession::new(document, WIDTH, HEIGHT).map_err(|error| error.to_string())
}

fn rows_document(scrolled: bool) -> RuntimeDocument {
    let id = DocumentId::new(1).expect("document id");
    let mut document = RuntimeDocument::new(id);
    let mut row_style = NodeStyle::default();
    {
        let layout = Arc::make_mut(&mut row_style.layout);
        layout.width = Some(LengthSpec::Px(WIDTH as f32));
        layout.height = Some(LengthSpec::Px(ROW_HEIGHT));
    }
    document
        .context_mut()
        .mount_view_root(id, || {
            let rows = (0..ROWS)
                .map(|row| widget(Text::new(format!("Row {row}")).style(row_style.clone())))
                .collect::<Vec<_>>();
            let column = widget(Stack::column(0.0)).children(rows);
            if scrolled {
                let mut viewport = ScrollView::new(ScrollAxes::Vertical);
                let layout = Arc::make_mut(&mut viewport.style.layout);
                layout.width = Some(LengthSpec::Px(WIDTH as f32));
                layout.height = Some(LengthSpec::Px(HEIGHT as f32));
                widget(viewport).children(column).into_any()
            } else {
                column.into_any()
            }
        })
        .expect("rows document");
    document
}

fn range_document() -> RuntimeDocument {
    let id = DocumentId::new(1).expect("document id");
    let mut document = RuntimeDocument::new(id);
    document
        .context_mut()
        .mount_view_root(id, || {
            let mut range = RangeField::new(0.5, 0.0, 1.0, 0.01);
            let layout = Arc::make_mut(&mut range.style.layout);
            layout.width = Some(LengthSpec::Px(400.0));
            widget(range)
        })
        .expect("range document");
    document
}

fn parse(argv: impl IntoIterator<Item = String>) -> Result<Args, String> {
    let mut args = Args {
        rounds: 15,
        events: 4000,
    };
    let mut argv = argv.into_iter();
    while let Some(name) = argv.next() {
        let value = argv
            .next()
            .ok_or_else(|| format!("{name} expects a value"))?;
        let value: usize = value
            .parse()
            .map_err(|_| format!("{name}: `{value}` is not a count"))?;
        match name.as_str() {
            "--rounds" if value > 0 => args.rounds = value,
            "--events" if value > 0 => args.events = value,
            _ => return Err(format!("unknown or zero argument {name}")),
        }
    }
    Ok(args)
}
