//! `.vue` views against the same views written as functions.
//!
//! A `.vue` file compiles to an ordinary `fn … -> impl IntoView`, so the
//! two can only differ where the compiler writes something other than what
//! a person would: folded constants, compiled stylesheets, hot-mode text.
//! Each scenario mounts or updates the same tree through every variant,
//! interleaved, rotating which goes first, and reports the minimum and the
//! median; then counts the live heap (allocations minus frees) and the
//! effects per row. `.vue hot` is the same file compiled as a debug build
//! compiles it, measured here in release so only the generated code
//! differs.
//!
//! `cargo run --release -p reactive-sfc --bin sfc-benchmark`

use std::alloc::{GlobalAlloc, Layout, System};
use std::any::Any;
use std::cell::Cell;
use std::hint::black_box;
use std::time::{Duration, Instant};

use nana_ui::runtime::view::{IntoView, Signal, column, reactive_stats, signal};
use nana_ui::runtime::{AppContext, DocumentId};
use reactive_sfc::bench::{Item, hot, idiomatic, inline_css, naive, views};

const ROUNDS: usize = 15;
const OPS: u32 = 20;

struct Counting;

thread_local! {
    static LIVE: Cell<i64> = const { Cell::new(0) };
    // The signal the last mount made, for the update scenarios.
    static SELECTED: Cell<Option<Signal<usize>>> = const { Cell::new(None) };
    static LIST: Cell<Option<Signal<Vec<Item>>>> = const { Cell::new(None) };
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

/// Mounts `n` rows of one variant into a context; the result keeps them.
type Mount = Box<dyn Fn(&mut AppContext, usize) -> Box<dyn Any>>;

/// One timed sample of a variant.
type Run<'a> = Box<dyn FnMut() -> Duration + 'a>;

fn mount(
    cx: &mut AppContext,
    view: impl FnOnce() -> nana_ui::runtime::view::AnyView,
) -> Box<dyn Any> {
    Box::new(
        cx.mount_view_root(DocumentId::new(1).unwrap(), view)
            .unwrap(),
    )
}

fn rows<V: IntoView + 'static>(row: fn(usize) -> V) -> Mount {
    Box::new(move |cx, n| {
        mount(cx, move || {
            column()
                .children((0..n).map(row).collect::<Vec<_>>())
                .into_any()
        })
    })
}

fn styled<V: IntoView + 'static>(row: fn(usize, Signal<usize>) -> V) -> Mount {
    Box::new(move |cx, n| {
        mount(cx, move || {
            let selected = signal(usize::MAX);
            SELECTED.set(Some(selected));
            column()
                .children((0..n).map(|i| row(i, selected)).collect::<Vec<_>>())
                .into_any()
        })
    })
}

fn list<V: IntoView + 'static>(view: fn(Signal<Vec<Item>>) -> V) -> Mount {
    Box::new(move |cx, n| {
        mount(cx, move || {
            let items = (0..n as u32)
                .map(|id| Item {
                    id,
                    title: signal(format!("任务 {id}")),
                })
                .collect();
            let list = signal(items);
            LIST.set(Some(list));
            view(list).into_any()
        })
    })
}

/// Run `runs` interleaved for [`ROUNDS`], rotating the first one.
fn interleave(scenario: &str, mut runs: Vec<(&str, Run<'_>)>) {
    let mut samples = vec![Vec::new(); runs.len()];
    for round in 0..ROUNDS {
        for offset in 0..runs.len() {
            let at = (round + offset) % runs.len();
            samples[at].push((runs[at].1)());
        }
    }
    for ((name, _), mut samples) in runs.into_iter().zip(samples) {
        samples.sort();
        let [min, median] = [samples[0], samples[samples.len() / 2]].map(|d| d.as_secs_f64() * 1e6);
        println!("{scenario:<34} {name:<14} min {min:>10.2} µs   median {median:>10.2} µs");
    }
}

fn mounts(scenario: &str, n: usize, variants: &[(&str, Mount)]) {
    let runs = variants
        .iter()
        .map(|(name, make)| {
            let run = move || {
                let mut cx = AppContext::typed();
                let started = Instant::now();
                black_box(make(&mut cx, n));
                started.elapsed()
            };
            (*name, Box::new(run) as Run)
        })
        .collect();
    interleave(scenario, runs);
}

/// Mount each variant once, then time `op(cx, handle, tick)` followed by a
/// flush, per op. `handle` reads what the mount left in a thread local.
fn updates<H: Copy + 'static>(
    scenario: &str,
    n: usize,
    variants: &[(&str, Mount)],
    handle: fn() -> H,
    op: fn(H, u32),
) {
    let runs = variants
        .iter()
        .map(|(name, make)| {
            let mut cx = AppContext::typed();
            let kept = make(&mut cx, n);
            let handle = handle();
            let mut tick = 0;
            let run = move || {
                let started = Instant::now();
                for _ in 0..OPS {
                    tick += 1;
                    op(handle, tick);
                    cx.flush_reactive().unwrap();
                }
                black_box(&kept);
                started.elapsed() / OPS
            };
            (*name, Box::new(run) as Run)
        })
        .collect();
    interleave(scenario, runs);
}

/// Live bytes and effects per row, after the thread's signal runtime, the
/// context's tables and the process-wide defaults have grown.
fn memory(n: usize, variants: &[(&str, Mount)]) {
    for (name, make) in variants {
        // Otherwise whichever variant runs first grows the thread's tables.
        drop(make(&mut AppContext::typed(), n));
        let mut cx = AppContext::typed();
        let warm = make(&mut cx, 8);
        let (live, effects) = (LIVE.get(), reactive_stats().effects);
        let kept = make(&mut cx, n);
        println!(
            "  {name:<14} live {:>6.0} B/row   effects {:.2}/row",
            (LIVE.get() - live) as f64 / n as f64,
            (reactive_stats().effects - effects) as f64 / n as f64,
        );
        drop((kept, warm, cx));
    }
}

fn load() -> String {
    std::process::Command::new("uptime")
        .output()
        .map(|out| String::from_utf8_lossy(&out.stdout).trim().to_owned())
        .unwrap_or_default()
}

fn main() {
    // A: static text, one part from a signal nothing writes.
    let a = [
        (".vue", rows(views::static_row)),
        ("idiomatic fn", rows(idiomatic::static_row)),
        ("naive fn", rows(naive::static_row)),
    ];
    // B: a stylesheet and one conditional class per row.
    let b = [
        (".vue <style>", styled(views::styled_row)),
        ("fn builders", styled(idiomatic::styled_row)),
        ("fn css!", styled(inline_css::styled_row)),
        (".vue hot", styled(hot::styled_row)),
    ];
    // C: a keyed list whose rows show a per-row signal.
    let c = [
        (".vue", list(views::row_list)),
        ("idiomatic fn", list(idiomatic::row_list)),
        (".vue hot", list(hot::row_list)),
    ];

    println!("load before: {}", load());
    println!("memory per row, 2000 rows");
    for (scenario, variants) in [("A", &a[..]), ("B", &b[..]), ("C", &c[..])] {
        println!(" {scenario}");
        memory(2_000, variants);
    }
    for n in [1_000, 5_000] {
        mounts(&format!("A mount {n} static rows"), n, &a);
    }
    for n in [1_000, 5_000] {
        mounts(&format!("B mount {n} styled rows"), n, &b[..3]);
        // Every row's condition re-runs, two rows change.
        let selected = || SELECTED.get().unwrap();
        updates(
            &format!("B switch class of {n} rows"),
            n,
            &b[..3],
            selected,
            |s, tick| s.set(tick as usize % 2 * 7 + 3),
        );
    }
    let n = 2_000;
    mounts(&format!("C mount {n}-row list"), n, &c);
    let list = || LIST.get().unwrap();
    updates(
        &format!("C insert or remove 1 row of {n}"),
        n,
        &c[..2],
        list,
        |list, tick| match tick % 2 {
            1 => {
                let (id, title) = (u32::MAX - tick, signal(format!("任务 {tick}")));
                list.update(|l| l.insert(0, Item { id, title }))
            }
            _ => list.update(|l| {
                l.remove(0);
            }),
        },
    );
    updates(
        &format!("C edit 1 row of {n}"),
        n,
        &c[..2],
        list,
        |list, tick| {
            list.with_untracked(|l| l[57].title)
                .set(format!("改 {tick}"))
        },
    );
    println!("load after:  {}", load());
}
