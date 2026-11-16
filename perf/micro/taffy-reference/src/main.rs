//! NanaUI layout beside its performance reference, [Taffy] 0.11.
//!
//! Taffy is the reference: a layout engine that keeps a style, a cache and a
//! box per node and nothing else. `compare` lays the same boxes out in both
//! (flex and grid) and reports the gap; `features` measures what
//! NanaUI does that Taffy has no counterpart for, NanaUI alone; `check`
//! confirms both engines put every box of the compared pages in the same
//! place. The numbers and what they mean are in
//! `docs/reference/taffy-reference.md`.
//!
//! ```text
//! cargo run --release -- compare            # 1000 rows (21 001 boxes), flex / grid
//! cargo run --release -- compare 100,1000   # row counts (10 cells a row)
//! cargo run --release -- features           # inline text, sticky / fixed, moves,
//!                                           # hit testing and accessibility, deep trees
//! cargo run --release -- check              # both engines place every box alike
//! ROUNDS=12 cargo run --release -- compare  # rounds to take the least of (8)
//! SHOW=1 cargo run --release -- check       # the first boxes that differ, if any
//! SHOW=1 cargo run --release -- features    # work counters and stages of the
//!                                           # inline edit and the scroll frames
//! SHOW=1 cargo run --release -- inline 100  # the inline text scene alone, at 100
//!                                           # paragraphs (1000 by default)
//! /usr/bin/time -l target/release/nana-taffy-reference reps flex nana 1000 5
//!                                           # a fixed run, to read instructions retired
//!                                           # from outside (build, layout and drop)
//! ```
//!
//! Every time is the least over `ROUNDS` rounds, NanaUI and Taffy taking
//! turns within each round (the order swaps every round): this machine's
//! load moves single runs several-fold. Instructions are read in process
//! from `proc_pid_rusage` (`ri_instructions`, what `time -l` prints as
//! "instructions retired"), also the least over the rounds, macOS only.
//! Heap is what the laid-out tree holds (allocations less frees), not
//! allocation traffic.
//!
//! [Taffy]: https://github.com/DioxusLabs/taffy

use std::alloc::{GlobalAlloc, Layout, System};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicIsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use nana_ui_core::{
    AlignSpec, DisplaySpec, FlexDirection, GridStyle, GridTrack, LayoutStyle, LengthSpec,
    OverflowSpec, PositionSpec, shared::Shared,
};
use nana_ui_runtime::{
    AppContext, ComputedStyle, DocumentId, Entity, FrameStage, LayoutViewport, MutationQueue,
    NanaTextEngineShaper, ScrollOffset, StableNodeId, Stack, Text, TextContent, TextMetrics,
    TextShapeConstraints, TextShaper,
};
use nana_ui_scene::RuntimeDocument;

/// Live heap bytes: allocations minus frees.
static LIVE: AtomicIsize = AtomicIsize::new(0);

struct Counting;

unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        LIVE.fetch_add(layout.size() as isize, Ordering::Relaxed);
        unsafe { System.alloc(layout) }
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        LIVE.fetch_add(layout.size() as isize, Ordering::Relaxed);
        unsafe { System.alloc_zeroed(layout) }
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
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

fn live() -> isize {
    LIVE.load(Ordering::Relaxed)
}

/// Instructions this process has retired so far (`ri_instructions` of
/// `proc_pid_rusage`, what `/usr/bin/time -l` reports), 0 elsewhere.
#[cfg(target_os = "macos")]
fn instructions() -> u64 {
    #[repr(C)]
    struct RusageInfo {
        uuid: [u8; 16],
        fields: [u64; 48],
    }
    unsafe extern "C" {
        fn proc_pid_rusage(pid: i32, flavor: i32, buffer: *mut RusageInfo) -> i32;
    }
    const RUSAGE_INFO_V4: i32 = 4;
    // `ri_instructions` is the 30th field after the UUID in
    // `rusage_info_v4` (`<sys/resource.h>`).
    const INSTRUCTIONS: usize = 29;
    let mut info = RusageInfo {
        uuid: [0; 16],
        fields: [0; 48],
    };
    let ok = unsafe { proc_pid_rusage(std::process::id() as i32, RUSAGE_INFO_V4, &mut info) };
    if ok == 0 {
        info.fields[INSTRUCTIONS]
    } else {
        0
    }
}

#[cfg(not(target_os = "macos"))]
fn instructions() -> u64 {
    0
}

fn rounds() -> usize {
    std::env::var("ROUNDS").map_or(8, |rounds| rounds.parse().unwrap())
}

fn us(start: Instant) -> f64 {
    start.elapsed().as_secs_f64() * 1e6
}

/// Text measured as 8 px a byte by 18 px: the box pages hold no text, and
/// a fixed measure keeps text out of what a box page compares.
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

/// The product text engine over the bundled UI face alone, as the
/// runtime's own tests shape: the same metrics on every machine.
fn bundled_face_shaper() -> NanaTextEngineShaper {
    use nana_text::NativeTextEngine;
    use nana_text::font::{FaceDescriptor, FallbackPolicy, FontSystem, GenericFamily};
    let mut policy = FallbackPolicy::empty();
    policy.set_generic(GenericFamily::SansSerif, ["Noto Sans SC"]);
    let mut fonts = FontSystem::with_policy(policy);
    fonts
        .register_file(
            concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/../../../crates/nana-ui/assets/fonts/NotoSansSC-Variable.ttf"
            ),
            &FaceDescriptor::default(),
        )
        .expect("the bundled UI face registers");
    NanaTextEngineShaper::new(Arc::new(Mutex::new(NativeTextEngine::new(fonts))))
}

// ---------------------------------------------------------------------------
// The compared pages
// ---------------------------------------------------------------------------

/// Which formatting context the page's containers use.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Kind {
    /// Flex: a column of rows, each row a line of cells with a 4 px gap.
    Flex,
    /// Grid: a column of rows, each row a grid of ten `1fr` columns with a
    /// 4 px column gap.
    Grid,
}

impl Kind {
    const ALL: [Self; 2] = [Self::Flex, Self::Grid];

    fn name(self) -> &'static str {
        match self {
            Self::Flex => "flex",
            Self::Grid => "grid",
        }
    }

    fn parse(name: &str) -> Self {
        match name {
            "flex" => Self::Flex,
            "grid" => Self::Grid,
            other => panic!("unknown layout {other}"),
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Role {
    Root,
    Row,
    Cell,
    Leaf,
    /// The column holding the rows under a header ([`spec_with_header`]).
    Body,
}

impl Role {
    const ALL: [Self; 5] = [Self::Root, Self::Row, Self::Cell, Self::Leaf, Self::Body];

    fn index(self) -> usize {
        Self::ALL.iter().position(|role| *role == self).unwrap()
    }
}

/// A page as a list of nodes, parents before children: `rows` rows of
/// `cells` cells under a 1280 px root, each cell holding one 24 × 16 leaf
/// in a quarter pixel of padding (`1 + rows * (1 + 2 * cells)` boxes, the
/// shape `perf/micro/stack-depth`'s `wide` lays out).
struct Spec {
    /// `(parent index, role)`; the root's parent is itself.
    nodes: Vec<(usize, Role)>,
    /// The middle row's sixth cell's leaf: what `height` edits.
    target: usize,
    /// A header leaf above the rows, when the page has one: what `move`
    /// edits.
    header: Option<usize>,
}

fn spec(rows: usize, cells: usize) -> Spec {
    rows_under(vec![(0, Role::Root)], 0, None, rows, cells)
}

/// [`spec`] with a header leaf first under the root and the rows in a
/// column below it: growing the header moves the column and everything in
/// it, and nothing in it changes size.
fn spec_with_header(rows: usize, cells: usize) -> Spec {
    rows_under(
        vec![(0, Role::Root), (0, Role::Leaf), (0, Role::Body)],
        2,
        Some(1),
        rows,
        cells,
    )
}

fn rows_under(
    mut nodes: Vec<(usize, Role)>,
    parent: usize,
    header: Option<usize>,
    rows: usize,
    cells: usize,
) -> Spec {
    let mut target = 0;
    for r in 0..rows {
        let row = nodes.len();
        nodes.push((parent, Role::Row));
        for c in 0..cells {
            let cell = nodes.len();
            nodes.push((row, Role::Cell));
            if (r, c) == (rows / 2, cells / 2) {
                target = nodes.len();
            }
            nodes.push((cell, Role::Leaf));
        }
    }
    Spec {
        nodes,
        target,
        header,
    }
}

/// The NanaUI style of a node, written out field by field as Taffy's below
/// so both engines read the same CSS.
fn nana_style(kind: Kind, role: Role) -> LayoutStyle {
    let flex = |direction| LayoutStyle {
        display: Some(DisplaySpec::Flex),
        direction: Some(direction),
        align_items: AlignSpec::Stretch,
        ..LayoutStyle::default()
    };
    let mut style = match (kind, role) {
        (Kind::Flex, Role::Row) => LayoutStyle {
            gap: Some(LengthSpec::Px(4.0)),
            ..flex(FlexDirection::Row)
        },
        (Kind::Grid, Role::Row) => LayoutStyle {
            display: Some(DisplaySpec::Grid),
            grid: Shared::new(GridStyle {
                columns: Some(vec![GridTrack::Fr(1.0); 10]),
                ..GridStyle::default()
            }),
            column_gap: Some(LengthSpec::Px(4.0)),
            ..LayoutStyle::default()
        },
        (Kind::Flex | Kind::Grid, _) => flex(FlexDirection::Column),
    };
    match role {
        Role::Root => style.width = Some(LengthSpec::Px(1280.0)),
        Role::Row | Role::Body => {}
        Role::Cell => style.padding = Some(LengthSpec::Px(0.25)),
        Role::Leaf => {
            style.width = Some(LengthSpec::Px(24.0));
            style.min_height = Some(LengthSpec::Px(16.0));
        }
    }
    style
}

fn taffy_style(kind: Kind, role: Role) -> taffy::Style {
    use taffy::prelude::*;
    let quarter = length(0.25_f32);
    let mut style = match (kind, role) {
        (Kind::Flex | Kind::Grid, Role::Leaf) => Style {
            display: Display::Block,
            ..Style::default()
        },
        (Kind::Flex, Role::Row) => Style {
            display: Display::Flex,
            flex_direction: FlexDirection::Row,
            gap: Size {
                width: length(4.0_f32),
                height: length(4.0_f32),
            },
            ..Style::default()
        },
        (Kind::Grid, Role::Row) => Style {
            display: Display::Grid,
            grid_template_columns: vec![fr(1.0_f32); 10],
            gap: Size {
                width: length(4.0_f32),
                height: zero(),
            },
            ..Style::default()
        },
        (Kind::Flex | Kind::Grid, _) => Style {
            display: Display::Flex,
            flex_direction: FlexDirection::Column,
            ..Style::default()
        },
    };
    match role {
        Role::Root => style.size.width = length(1280.0_f32),
        Role::Row | Role::Body => {}
        Role::Cell => {
            style.padding = Rect {
                left: quarter,
                right: quarter,
                top: quarter,
                bottom: quarter,
            }
        }
        Role::Leaf => {
            style.size.width = length(24.0_f32);
            style.min_size.height = length(16.0_f32);
        }
    }
    style
}

/// Builds the page in `cx`; returns each node's id, in the spec's order.
fn nana_build(
    cx: &mut AppContext,
    document: DocumentId,
    kind: Kind,
    spec: &Spec,
) -> Vec<StableNodeId> {
    nana_build_entities(cx, document, kind, spec)
        .iter()
        .map(|entity| entity.stable_id())
        .collect()
}

fn nana_build_entities(
    cx: &mut AppContext,
    document: DocumentId,
    kind: Kind,
    spec: &Spec,
) -> Vec<Entity<Stack>> {
    // One shared layout per role, as a page's repeated rows share theirs.
    let styles: Vec<Arc<LayoutStyle>> = Role::ALL
        .into_iter()
        .map(|role| Arc::new(nana_style(kind, role)))
        .collect();
    let style = |role: Role| Arc::clone(&styles[role.index()]);
    let mut entities: Vec<Entity<Stack>> = Vec::with_capacity(spec.nodes.len());
    for (index, &(parent, role)) in spec.nodes.iter().enumerate() {
        let node = Stack::from_layout(style(role));
        let entity = if index == 0 {
            cx.create_component(document, node).unwrap()
        } else {
            let entity = cx.create_detached_component(document, node).unwrap();
            cx.append_child(entities[parent], entity).unwrap();
            entity
        };
        entities.push(entity);
    }
    entities
}

fn taffy_build(kind: Kind, spec: &Spec) -> (taffy::TaffyTree<()>, Vec<taffy::NodeId>) {
    let mut tree: taffy::TaffyTree<()> = taffy::TaffyTree::with_capacity(spec.nodes.len());
    // NanaUI keeps fractional pixels; so does Taffy here.
    tree.disable_rounding();
    let styles: Vec<taffy::Style> = Role::ALL
        .into_iter()
        .map(|role| taffy_style(kind, role))
        .collect();
    let mut ids = Vec::with_capacity(spec.nodes.len());
    let mut children: Vec<Vec<taffy::NodeId>> = vec![Vec::new(); spec.nodes.len()];
    for (index, &(parent, role)) in spec.nodes.iter().enumerate() {
        let style = styles[role.index()].clone();
        let id = tree.new_leaf(style).unwrap();
        if index != 0 {
            children[parent].push(id);
        }
        ids.push(id);
    }
    for (index, children) in children.iter().enumerate() {
        if !children.is_empty() {
            tree.set_children(ids[index], children).unwrap();
        }
    }
    (tree, ids)
}

const VIEWPORT: (f32, f32) = (1280.0, 800.0);

fn viewport() -> LayoutViewport {
    LayoutViewport::new(VIEWPORT.0, VIEWPORT.1)
}

fn taffy_available() -> taffy::Size<taffy::AvailableSpace> {
    taffy::Size {
        width: taffy::AvailableSpace::Definite(VIEWPORT.0),
        height: taffy::AvailableSpace::Definite(VIEWPORT.1),
    }
}

/// One cold run: layout time, build-and-layout time (µs), heap the laid-out
/// page holds (B), and the instructions the layout and the build and layout
/// retired.
#[derive(Clone, Copy)]
struct Cold {
    layout: f64,
    total: f64,
    held: isize,
    layout_instructions: u64,
    total_instructions: u64,
}

impl Cold {
    fn least(self, other: Self) -> Self {
        Self {
            layout: self.layout.min(other.layout),
            total: self.total.min(other.total),
            held: other.held,
            layout_instructions: self.layout_instructions.min(other.layout_instructions),
            total_instructions: self.total_instructions.min(other.total_instructions),
        }
    }

    const NONE: Self = Self {
        layout: f64::INFINITY,
        total: f64::INFINITY,
        held: 0,
        layout_instructions: u64::MAX,
        total_instructions: u64::MAX,
    };
}

/// NanaUI's first layout of the page, as the product frame runs it
/// (`layout_document_for_viewport` on a document never laid out).
fn nana_cold(kind: Kind, spec: &Spec) -> Cold {
    let document = DocumentId::new(1).unwrap();
    let before = live();
    let built = Instant::now();
    let built_instructions = instructions();
    let mut cx = AppContext::new();
    nana_build(&mut cx, document, kind, spec);
    let start = Instant::now();
    let start_instructions = instructions();
    cx.layout_document_for_viewport(document, viewport(), &[])
        .unwrap();
    let end_instructions = instructions();
    let cold = Cold {
        layout: us(start),
        total: us(built),
        held: live() - before,
        layout_instructions: end_instructions - start_instructions,
        total_instructions: end_instructions - built_instructions,
    };
    drop(cx);
    cold
}

fn taffy_cold(kind: Kind, spec: &Spec) -> Cold {
    let before = live();
    let built = Instant::now();
    let built_instructions = instructions();
    let (mut tree, ids) = taffy_build(kind, spec);
    let start = Instant::now();
    let start_instructions = instructions();
    tree.compute_layout(ids[0], taffy_available()).unwrap();
    let end_instructions = instructions();
    let cold = Cold {
        layout: us(start),
        total: us(built),
        held: live() - before,
        layout_instructions: end_instructions - start_instructions,
        total_instructions: end_instructions - built_instructions,
    };
    drop(tree);
    cold
}

/// An edit's least times (µs) and instructions over its runs.
#[derive(Clone, Copy)]
struct Edit {
    /// NanaUI: the frame's layout stage; Taffy: `compute_layout`.
    layout: f64,
    /// NanaUI: the whole product frame (`RuntimeDocument::flush`: layout,
    /// hit index, accessibility, extraction); Taffy: the same as `layout`.
    frame: f64,
    instructions: u64,
}

impl Edit {
    const NONE: Self = Self {
        layout: f64::INFINITY,
        frame: f64::INFINITY,
        instructions: u64::MAX,
    };

    fn least(self, other: Self) -> Self {
        Self {
            layout: self.layout.min(other.layout),
            frame: self.frame.min(other.frame),
            instructions: self.instructions.min(other.instructions),
        }
    }
}

/// The edits: `height` toggles the target leaf's minimum height (16 or
/// 24 px; its row grows and every row below moves), `move` toggles the
/// header's minimum height on a [`spec_with_header`] page (16 or 23.37 px;
/// the column of rows moves, nothing in it changes size).
#[derive(Clone, Copy, PartialEq, Eq)]
enum EditKind {
    Height,
    Move,
}

/// A laid-out NanaUI page in a product document, edited a frame at a time.
struct NanaEdits {
    runtime: RuntimeDocument,
    ids: Vec<StableNodeId>,
    target: usize,
    header: Option<usize>,
    on: bool,
}

impl NanaEdits {
    fn new(kind: Kind, spec: &Spec) -> Self {
        let document = DocumentId::new(1).unwrap();
        let mut runtime = RuntimeDocument::new(document);
        let ids = nana_build(runtime.context_mut(), document, kind, spec);
        runtime.flush(viewport(), &mut FixedShaper).unwrap();
        Self {
            runtime,
            ids,
            target: spec.target,
            header: spec.header,
            on: false,
        }
    }

    fn edit(&mut self, edit: EditKind) -> Edit {
        self.on = !self.on;
        let on = self.on;
        let id = match edit {
            EditKind::Height => self.ids[self.target],
            EditKind::Move => self.ids[self.header.expect("a page with a header")],
        };
        let cx = self.runtime.context_mut();
        let mut style = cx.world().node_style(id).unwrap().clone();
        let layout = Arc::make_mut(&mut style.layout);
        match edit {
            EditKind::Height => {
                layout.min_height = Some(LengthSpec::Px(if on { 24.0 } else { 16.0 }))
            }
            EditKind::Move => {
                layout.min_height = Some(LengthSpec::Px(if on { 23.37 } else { 16.0 }))
            }
        }
        let mut queue = MutationQueue::new();
        queue.set_style(id, style);
        cx.commit_mutations(queue).unwrap();
        let start = Instant::now();
        let start_instructions = instructions();
        self.runtime.flush(viewport(), &mut FixedShaper).unwrap();
        let instructions = instructions() - start_instructions;
        let frame = us(start);
        Edit {
            layout: stage(&self.runtime, FrameStage::Layout),
            frame,
            instructions,
        }
    }
}

/// The least of a frame's stage, in µs.
fn stage(runtime: &RuntimeDocument, stage: FrameStage) -> f64 {
    runtime
        .context()
        .last_frame_profile()
        .stages
        .iter()
        .filter(|timing| timing.stage == stage)
        .map(|timing| timing.duration.as_secs_f64() * 1e6)
        .sum()
}

struct TaffyEdits {
    tree: taffy::TaffyTree<()>,
    ids: Vec<taffy::NodeId>,
    target: usize,
    header: Option<usize>,
    on: bool,
}

impl TaffyEdits {
    fn new(kind: Kind, spec: &Spec) -> Self {
        let (mut tree, ids) = taffy_build(kind, spec);
        tree.compute_layout(ids[0], taffy_available()).unwrap();
        Self {
            tree,
            ids,
            target: spec.target,
            header: spec.header,
            on: false,
        }
    }

    /// `set_style` (which marks the node and its ancestors dirty, as
    /// `mark_dirty` does) and `compute_layout`.
    fn edit(&mut self, edit: EditKind) -> Edit {
        use taffy::prelude::*;
        self.on = !self.on;
        let on = self.on;
        let id = match edit {
            EditKind::Height => self.ids[self.target],
            EditKind::Move => self.ids[self.header.expect("a page with a header")],
        };
        let start = Instant::now();
        let start_instructions = instructions();
        let mut style = self.tree.style(id).unwrap().clone();
        match edit {
            EditKind::Height => style.min_size.height = length(if on { 24.0_f32 } else { 16.0 }),
            EditKind::Move => {
                style.min_size.height = length(if on { 23.37_f32 } else { 16.0 });
            }
        }
        self.tree.set_style(id, style).unwrap();
        self.tree
            .compute_layout(self.ids[0], taffy_available())
            .unwrap();
        let instructions = instructions() - start_instructions;
        let took = us(start);
        Edit {
            layout: took,
            frame: took,
            instructions,
        }
    }
}

fn boxes(rows: usize) -> usize {
    1 + rows * (1 + 2 * 10)
}

fn compare(rows: &[usize], kinds: &[Kind]) {
    let rounds = rounds();
    println!(
        "Taffy 0.11 is the reference. Least of {rounds} rounds, the engines taking turns ({}).\n",
        if cfg!(debug_assertions) {
            "debug"
        } else {
            "release"
        }
    );
    println!(
        "| layout | boxes | engine | layout µs / box | build + layout µs / box | held B / box | layout instr / box | build + layout instr / box |"
    );
    println!("| --- | ---: | --- | ---: | ---: | ---: | ---: | ---: |");
    let mut edits_rows = Vec::new();
    let largest = *rows.iter().max().unwrap();
    for &rows in rows {
        let spec = spec(rows, 10);
        let boxes = boxes(rows) as f64;
        for &kind in kinds {
            let (mut nana, mut taffy) = (Cold::NONE, Cold::NONE);
            for round in 0..rounds {
                if round % 2 == 0 {
                    nana = nana.least(nana_cold(kind, &spec));
                    taffy = taffy.least(taffy_cold(kind, &spec));
                } else {
                    taffy = taffy.least(taffy_cold(kind, &spec));
                    nana = nana.least(nana_cold(kind, &spec));
                }
            }
            for (engine, cold) in [("NanaUI", nana), ("Taffy", taffy)] {
                println!(
                    "| {} | {boxes} | {engine} | {:.2} | {:.2} | {:.0} | {:.1} k | {:.1} k |",
                    kind.name(),
                    cold.layout / boxes,
                    cold.total / boxes,
                    cold.held as f64 / boxes,
                    cold.layout_instructions as f64 / boxes / 1e3,
                    cold.total_instructions as f64 / boxes / 1e3,
                );
            }
            println!(
                "| {} | {boxes} | NanaUI ÷ Taffy | {:.2}× | {:.2}× | {:.2}× | {:.2}× | {:.2}× |",
                kind.name(),
                nana.layout / taffy.layout,
                nana.total / taffy.total,
                nana.held as f64 / taffy.held as f64,
                nana.layout_instructions as f64 / taffy.layout_instructions as f64,
                nana.total_instructions as f64 / taffy.total_instructions as f64,
            );
            // Edits on the largest page only.
            if rows == largest {
                let mut nana_page = NanaEdits::new(kind, &spec);
                let mut taffy_page = TaffyEdits::new(kind, &spec);
                let (mut nana, mut taffy) = (Edit::NONE, Edit::NONE);
                for round in 0..rounds {
                    for _ in 0..10 {
                        if round % 2 == 0 {
                            nana = nana.least(nana_page.edit(EditKind::Height));
                            taffy = taffy.least(taffy_page.edit(EditKind::Height));
                        } else {
                            taffy = taffy.least(taffy_page.edit(EditKind::Height));
                            nana = nana.least(nana_page.edit(EditKind::Height));
                        }
                    }
                }
                edits_rows.push((kind, boxes, nana, taffy));
            }
        }
    }
    println!(
        "\nIncremental: `height` toggles one leaf's min-height in the middle row (its row grows, every row below moves).\n"
    );
    println!(
        "| layout | boxes | NanaUI layout µs | NanaUI frame µs | NanaUI instr / edit | Taffy layout µs | Taffy instr / edit | layout ÷ | instr ÷ |"
    );
    println!("| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |");
    for (kind, boxes, nana, taffy) in edits_rows {
        println!(
            "| {} | {boxes} | {:.1} | {:.1} | {:.2} M | {:.1} | {:.2} M | {:.2}× | {:.2}× |",
            kind.name(),
            nana.layout,
            nana.frame,
            nana.instructions as f64 / 1e6,
            taffy.layout,
            taffy.instructions as f64 / 1e6,
            nana.layout / taffy.layout,
            nana.instructions as f64 / taffy.instructions as f64,
        );
    }
}

/// Both engines place every box of every compared page in the same place.
fn check(rows: usize) {
    for (name, spec, kind) in Kind::ALL
        .into_iter()
        .map(|kind| ("", spec(rows, 10), kind))
        .chain(std::iter::once((
            " with a header",
            spec_with_header(rows, 10),
            Kind::Flex,
        )))
    {
        let document = DocumentId::new(1).unwrap();
        let mut cx = AppContext::new();
        let nana = nana_build(&mut cx, document, kind, &spec);
        cx.layout_document_for_viewport(document, viewport(), &[])
            .unwrap();
        let (mut tree, ids) = taffy_build(kind, &spec);
        tree.compute_layout(ids[0], taffy_available()).unwrap();
        // Taffy's locations are relative to the parent; sum them down.
        let mut page: Vec<(f32, f32)> = vec![(0.0, 0.0); ids.len()];
        let mut worst = 0.0_f32;
        let mut off = 0;
        for (index, &(parent, _)) in spec.nodes.iter().enumerate() {
            let layout = tree.layout(ids[index]).unwrap();
            let origin = if index == 0 { (0.0, 0.0) } else { page[parent] };
            page[index] = (origin.0 + layout.location.x, origin.1 + layout.location.y);
            let ours = cx.world().layout_box(nana[index]).unwrap();
            let diff = (ours.x - page[index].0)
                .abs()
                .max((ours.y - page[index].1).abs())
                .max((ours.width - layout.size.width).abs())
                .max((ours.height - layout.size.height).abs());
            if diff > 0.01 {
                if off < 4 && std::env::var_os("SHOW").is_some() {
                    println!(
                        "  #{index}: NanaUI {:?}, Taffy ({}, {}) {} × {}",
                        ours, page[index].0, page[index].1, layout.size.width, layout.size.height
                    );
                }
                off += 1;
            }
            worst = worst.max(diff);
        }
        let root = tree.layout(ids[0]).unwrap();
        println!(
            "{}{name}: {} boxes, {off} differ by more than 0.01 px, worst {worst:.4} px; root {} × {}",
            kind.name(),
            ids.len(),
            root.size.width,
            root.size.height
        );
    }
}

// ---------------------------------------------------------------------------
// What Taffy has no counterpart for
// ---------------------------------------------------------------------------

fn least_of(rounds: usize, mut run: impl FnMut() -> f64) -> f64 {
    (0..rounds).map(|_| run()).fold(f64::INFINITY, f64::min)
}

/// Inline text: paragraphs of text runs and inline-block boxes, broken into
/// lines by nana-text and aligned on their baselines. `rows` paragraphs,
/// each a 400 px block holding four runs and two inline-blocks.
fn inline_page(
    cx: &mut AppContext,
    document: DocumentId,
    rows: usize,
) -> (Vec<StableNodeId>, Vec<StableNodeId>) {
    let root = cx
        .create_component(
            document,
            Stack::from_layout(LayoutStyle {
                display: Some(DisplaySpec::Flex),
                width: Some(LengthSpec::Px(1280.0)),
                ..LayoutStyle::default()
            }),
        )
        .unwrap();
    let paragraph = Arc::new(LayoutStyle {
        display: Some(DisplaySpec::Flex),
        width: Some(LengthSpec::Px(400.0)),
        ..LayoutStyle::default()
    });
    let atom = Arc::new(LayoutStyle {
        display: Some(DisplaySpec::InlineFlex),
        width: Some(LengthSpec::Px(24.0)),
        height: Some(LengthSpec::Px(16.0)),
        ..LayoutStyle::default()
    });
    let words = [
        "The quick brown fox jumps over",
        "the lazy dog while 敏捷的狐狸",
        "跳过了懒狗, and the line wraps",
        "where the paragraph runs out of room.",
    ];
    let mut paragraphs = Vec::with_capacity(rows);
    let mut runs = Vec::with_capacity(rows);
    for _ in 0..rows {
        let p = cx
            .create_detached_component(document, Stack::from_layout(Arc::clone(&paragraph)))
            .unwrap();
        cx.append_child(root, p).unwrap();
        paragraphs.push(p.stable_id());
        for (index, words) in words.iter().enumerate() {
            let mut run = Text::new(*words);
            Arc::make_mut(&mut run.style.layout).display = Some(DisplaySpec::Inline);
            let run = cx.create_detached_component(document, run).unwrap();
            cx.append_child(p, run).unwrap();
            if index == 1 {
                runs.push(run.stable_id());
            }
            if index % 2 == 0 {
                let a = cx
                    .create_detached_component(document, Stack::from_layout(Arc::clone(&atom)))
                    .unwrap();
                cx.append_child(p, a).unwrap();
            }
        }
    }
    (paragraphs, runs)
}

fn inline_text(rows: usize, rounds: usize) {
    let document = DocumentId::new(1).unwrap();
    let mut boxes = 0;
    let mut cold = (f64::INFINITY, f64::INFINITY, 0_isize, u64::MAX);
    let mut lines = 0.0;
    for _ in 0..rounds.min(5) {
        let mut shaper = bundled_face_shaper();
        let before = live();
        let mut runtime = RuntimeDocument::new(document);
        let (paragraphs, _) = inline_page(runtime.context_mut(), document, rows);
        boxes = 1 + rows * 7;
        let start = Instant::now();
        let start_instructions = instructions();
        runtime.flush(viewport(), &mut shaper).unwrap();
        let spent = instructions() - start_instructions;
        let frame = us(start);
        cold = (
            cold.0.min(frame),
            cold.1.min(stage(&runtime, FrameStage::Layout)),
            live() - before,
            cold.3.min(spent),
        );
        let p = runtime.context().world().layout_box(paragraphs[0]).unwrap();
        lines = p.height;
        drop(runtime);
    }
    let mut shaper = bundled_face_shaper();
    let mut runtime = RuntimeDocument::new(document);
    let (_, runs) = inline_page(runtime.context_mut(), document, rows);
    runtime.flush(viewport(), &mut shaper).unwrap();
    let target = runs[rows / 2];
    let mut edit = Edit::NONE;
    for run in 0..rounds * 10 {
        let value = if run % 2 == 0 {
            "the lazy dog, a sleepy cat and 敏捷的狐狸"
        } else {
            "the lazy dog while 敏捷的狐狸"
        };
        let mut queue = MutationQueue::new();
        queue.set_text(
            target,
            TextContent {
                value: value.into(),
            },
        );
        runtime.context_mut().commit_mutations(queue).unwrap();
        let start = Instant::now();
        let start_instructions = instructions();
        runtime.flush(viewport(), &mut shaper).unwrap();
        let spent = instructions() - start_instructions;
        edit = edit.least(Edit {
            layout: stage(&runtime, FrameStage::Layout),
            frame: us(start),
            instructions: spent,
        });
    }
    if std::env::var_os("SHOW").is_some() {
        let f = runtime.context().layout_frontier_stats(document);
        eprintln!(
            "inline edit: seeds {} contexts {} measure {} place {} full {} local {}",
            f.seeds,
            f.contexts,
            f.nodes_measure,
            f.nodes_placement,
            f.full_document_fallbacks,
            f.local_subtree_fallbacks,
        );
        for timing in &runtime.context().last_frame_profile().stages {
            eprintln!(
                "  {:?} {:.0} µs",
                timing.stage,
                timing.duration.as_secs_f64() * 1e6
            );
        }
    }
    let b = boxes as f64;
    println!(
        "| inline text: {rows} paragraphs of 4 runs + 2 inline-blocks, 400 px wide ({boxes} boxes, {lines} px tall each) | cold frame {:.2} µs / box (layout stage {:.2}), {:.0} B / box held, {:.1} k instr / box | one run's text changed: layout {:.1} µs, frame {:.1} µs, {:.2} M instr |",
        cold.0 / b,
        cold.1 / b,
        cold.2 as f64 / b,
        cold.3 as f64 / b / 1e3,
        edit.layout,
        edit.frame,
        edit.instructions as f64 / 1e6,
    );
}

/// The flex page in an 800 px scroller, every tenth row `position: sticky;
/// top: 0` and twenty `position: fixed` badges; scrolled once a frame.
/// Beside the same page without sticky rows or fixed badges.
fn sticky_fixed(rows: usize, rounds: usize) {
    let spec = spec(rows, 10);
    let document = DocumentId::new(1).unwrap();
    let page = |positioned: bool| {
        let mut runtime = RuntimeDocument::new(document);
        let cx = runtime.context_mut();
        let entities = nana_build_entities(cx, document, Kind::Flex, &spec);
        let ids: Vec<StableNodeId> = entities.iter().map(|entity| entity.stable_id()).collect();
        let root = ids[0];
        let mut queue = MutationQueue::new();
        let mut style = cx.world().node_style(root).unwrap().clone();
        {
            let layout = Arc::make_mut(&mut style.layout);
            layout.height = Some(LengthSpec::Px(800.0));
            layout.overflow_y = OverflowSpec::Auto;
        }
        queue.set_style(root, style);
        if positioned {
            for (row, index) in spec
                .nodes
                .iter()
                .enumerate()
                .filter(|(_, (_, role))| *role == Role::Row)
                .map(|(index, _)| index)
                .enumerate()
            {
                if row % 10 == 0 {
                    let mut style = cx.world().node_style(ids[index]).unwrap().clone();
                    let layout = Arc::make_mut(&mut style.layout);
                    layout.position = PositionSpec::Sticky;
                    layout.offset_top = Some(LengthSpec::Px(0.0));
                    queue.set_style(ids[index], style);
                }
            }
        }
        cx.commit_mutations(queue).unwrap();
        if positioned {
            let root_entity = entities[0];
            for badge in 0..20 {
                let badge = cx
                    .create_detached_component(
                        document,
                        Stack::from_layout(LayoutStyle {
                            display: Some(DisplaySpec::Flex),
                            position: PositionSpec::Fixed,
                            offset_top: Some(LengthSpec::Px(8.0 + 30.0 * badge as f32)),
                            offset_right: Some(LengthSpec::Px(8.0)),
                            width: Some(LengthSpec::Px(24.0)),
                            height: Some(LengthSpec::Px(24.0)),
                            ..LayoutStyle::default()
                        }),
                    )
                    .unwrap();
                cx.append_child(root_entity, badge).unwrap();
            }
        }
        let start = Instant::now();
        runtime.flush(viewport(), &mut FixedShaper).unwrap();
        let cold = us(start);
        let cold_layout = stage(&runtime, FrameStage::Layout);
        (runtime, root, cold, cold_layout)
    };
    let scroll = |runtime: &mut RuntimeDocument, root: StableNodeId, y: f32| {
        let mut queue = MutationQueue::new();
        queue.set_scroll_offset(root, ScrollOffset { x: 0.0, y });
        runtime.context_mut().commit_mutations(queue).unwrap();
        let start = Instant::now();
        let start_instructions = instructions();
        runtime.flush(viewport(), &mut FixedShaper).unwrap();
        let flushed = us(start);
        // Cull as a painter does: the scene places sticky boxes by the
        // scroll here.
        let rect = nana_ui_scene::SceneRect {
            x: 0.0,
            y: 0.0,
            width: VIEWPORT.0,
            height: VIEWPORT.1,
        };
        std::hint::black_box(runtime.scene().visible_operations(rect).unwrap());
        let spent = instructions() - start_instructions;
        if std::env::var_os("SHOW").is_some() {
            eprintln!(
                "scroll {y}: offset {:?}",
                runtime.context().world().scroll_offset(root),
            );
            for timing in &runtime.context().last_frame_profile().stages {
                if !timing.duration.is_zero() {
                    eprintln!(
                        "  {:?} {:.0} µs",
                        timing.stage,
                        timing.duration.as_secs_f64() * 1e6
                    );
                }
            }
        }
        // `layout` carries the flush alone here, `frame` the flush and
        // the cull.
        Edit {
            layout: flushed,
            frame: us(start),
            instructions: spent,
        }
    };
    let mut results = [(f64::INFINITY, f64::INFINITY, Edit::NONE); 2];
    for round in 0..rounds {
        let order = if round % 2 == 0 {
            [true, false]
        } else {
            [false, true]
        };
        for positioned in order {
            let (mut runtime, root, cold, cold_layout) = page(positioned);
            let slot = &mut results[positioned as usize];
            slot.0 = slot.0.min(cold);
            slot.1 = slot.1.min(cold_layout);
            for step in 0..10 {
                // A quarter and a half of the way down.
                let range = rows as f32 * 16.5 - 800.0;
                let y = range * (0.25 + 0.25 * (step % 2) as f32);
                slot.2 = slot.2.least(scroll(&mut runtime, root, y));
            }
        }
    }
    let b = boxes(rows) as f64;
    for (name, (cold, cold_layout, scrolled)) in [
        (
            "sticky + fixed: the flex page in an 800 px scroller, every tenth row sticky, 20 fixed badges",
            results[1],
        ),
        (
            "the same scroller without sticky rows or fixed badges",
            results[0],
        ),
    ] {
        println!(
            "| {name} | cold frame {:.2} µs / box (layout stage {:.2}) | one scroll: frame {:.1} µs, with a painter's cull {:.1} µs, {:.2} M instr |",
            cold / b,
            cold_layout / b,
            scrolled.layout,
            scrolled.frame,
            scrolled.instructions as f64 / 1e6,
        );
    }
}

/// Retained relative geometry: a header above the rows grows, so the column
/// holding every row moves and nothing in it changes size. NanaUI keeps
/// boxes relative to their frame (`abs = frame + local`) and moves the
/// column by its one offset, whatever it holds; Taffy, also relative,
/// re-runs the root over a cached column. Run at a tenth of the page and
/// the whole: the cost should not follow the boxes moved.
fn moves(rows: usize, rounds: usize) {
    for rows in [rows / 10, rows] {
        let spec = spec_with_header(rows, 10);
        let mut nana_page = NanaEdits::new(Kind::Flex, &spec);
        let mut taffy_page = TaffyEdits::new(Kind::Flex, &spec);
        let (mut nana, mut taffy) = (Edit::NONE, Edit::NONE);
        for round in 0..rounds {
            for _ in 0..10 {
                if round % 2 == 0 {
                    nana = nana.least(nana_page.edit(EditKind::Move));
                    taffy = taffy.least(taffy_page.edit(EditKind::Move));
                } else {
                    taffy = taffy.least(taffy_page.edit(EditKind::Move));
                    nana = nana.least(nana_page.edit(EditKind::Move));
                }
            }
        }
        let f = nana_page
            .runtime
            .context()
            .layout_frontier_stats(DocumentId::new(1).unwrap());
        println!(
            "| move: a header above the rows grows; the column below moves with {} boxes in it | NanaUI layout {:.1} µs, frame {:.1} µs (layout, hit index, accessibility, scene), {:.3} M instr; frontier {} measure / {} place | Taffy layout {:.1} µs, {:.3} M instr |",
            spec.nodes.len() - 3,
            nana.layout,
            nana.frame,
            nana.instructions as f64 / 1e6,
            f.nodes_measure,
            f.nodes_placement,
            taffy.layout,
            taffy.instructions as f64 / 1e6,
        );
    }
}

/// What the product frame builds beside the layout (hit index,
/// accessibility, extraction and the scene), its heap, and a query of each.
fn hit_and_accessibility(rows: usize, rounds: usize) {
    let spec = spec(rows, 10);
    let document = DocumentId::new(1).unwrap();
    let b = boxes(rows) as f64;
    let mut stages = [f64::INFINITY; 4];
    let mut held = (0_isize, 0_isize, 0_isize);
    let (mut hit, mut a11y) = (f64::INFINITY, f64::INFINITY);
    let names = [
        FrameStage::Layout,
        FrameStage::HitTest,
        FrameStage::Accessibility,
        FrameStage::Extract,
    ];
    for _ in 0..rounds {
        // The tree alone, then laid out.
        let before = live();
        let mut cx = AppContext::new();
        nana_build(&mut cx, document, Kind::Flex, &spec);
        let built = live() - before;
        cx.layout_document_for_viewport(document, viewport(), &[])
            .unwrap();
        let layout_only = live() - before;
        drop(cx);
        // The product frame.
        let before = live();
        let mut runtime = RuntimeDocument::new(document);
        nana_build(runtime.context_mut(), document, Kind::Flex, &spec);
        runtime.flush(viewport(), &mut FixedShaper).unwrap();
        held = (built, layout_only, live() - before);
        for (slot, name) in stages.iter_mut().zip(names) {
            *slot = slot.min(stage(&runtime, name));
        }
        let world = runtime.context().world();
        // 1 000 points over the visible page.
        let start = Instant::now();
        let mut found = 0;
        for i in 0..1_000 {
            let x = (i * 37 % 1280) as f32 + 0.5;
            let y = (i * 53 % 800) as f32 + 0.5;
            found += world.hit_test(document, x, y).is_some() as usize;
        }
        hit = hit.min(us(start) / 1_000.0);
        std::hint::black_box(found);
        let start = Instant::now();
        let tree = world.project_accessibility(document);
        a11y = a11y.min(us(start) / tree.len().max(1) as f64);
        drop(tree);
        drop(runtime);
    }
    println!(
        "| hit testing + accessibility: the flex page's first product frame ({} boxes) | stages per box: layout {:.2}, hit index {:.2}, accessibility {:.2}, extract {:.2} µs; held {:.0} B / box (the tree before layout {:.0}, laid out {:.0}; `LayoutResult` {} B) | a hit test {:.2} µs; projecting the whole accessibility tree {:.3} µs / node |",
        boxes(rows),
        stages[0] / b,
        stages[1] / b,
        stages[2] / b,
        stages[3] / b,
        held.2 as f64 / b,
        held.0 as f64 / b,
        held.1 as f64 / b,
        std::mem::size_of::<nana_ui_runtime::LayoutResult>(),
        hit,
        a11y,
    );
}

/// A chain `depth` flex columns deep, laid out by `engine` (`nana`: a whole
/// product frame; `taffy`: `compute_layout`). Returns µs.
fn deep(engine: &str, depth: usize) -> f64 {
    let start = Instant::now();
    match engine {
        "nana" => {
            let document = DocumentId::new(1).unwrap();
            let mut runtime = RuntimeDocument::new(document);
            let cx = runtime.context_mut();
            let column = Arc::new(LayoutStyle {
                display: Some(DisplaySpec::Flex),
                direction: Some(FlexDirection::Column),
                padding: Some(LengthSpec::Px(0.25)),
                ..LayoutStyle::default()
            });
            let mut parent = cx
                .create_component(
                    document,
                    Stack::from_layout(LayoutStyle {
                        width: Some(LengthSpec::Px(800.0)),
                        ..(*column).clone()
                    }),
                )
                .unwrap();
            for _ in 0..depth {
                let child = cx
                    .create_detached_component(document, Stack::from_layout(Arc::clone(&column)))
                    .unwrap();
                cx.append_child(parent, child).unwrap();
                parent = child;
            }
            runtime.flush(viewport(), &mut FixedShaper).unwrap();
        }
        "taffy" => {
            use taffy::prelude::*;
            let mut tree: TaffyTree<()> = TaffyTree::new();
            let quarter = length(0.25_f32);
            let column = Style {
                flex_direction: FlexDirection::Column,
                padding: Rect {
                    left: quarter,
                    right: quarter,
                    top: quarter,
                    bottom: quarter,
                },
                ..Default::default()
            };
            let mut node = tree.new_leaf(column.clone()).unwrap();
            for _ in 0..depth {
                node = tree.new_with_children(column.clone(), &[node]).unwrap();
            }
            tree.compute_layout(node, taffy_available()).unwrap();
        }
        other => panic!("unknown engine {other}"),
    }
    us(start)
}

/// Whether `engine` lays a chain `depth` deep out on a thread of `stack`
/// bytes, in a child process (an overflow aborts the process).
fn fits(engine: &str, depth: usize, stack: usize) -> bool {
    Command::new(std::env::current_exe().unwrap())
        .args(["probe", engine, &depth.to_string(), &stack.to_string()])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|status| status.success())
}

fn deep_trees(rounds: usize) {
    const STACK: usize = 512 << 10;
    for depth in [200, 500] {
        let line: Vec<String> = ["nana", "taffy"]
            .iter()
            .map(|engine| {
                let fits = fits(engine, depth, STACK);
                let time = if fits {
                    let engine = engine.to_string();
                    let took = std::thread::Builder::new()
                        .stack_size(STACK)
                        .spawn(move || least_of(rounds, || deep(&engine, depth)))
                        .unwrap()
                        .join()
                        .unwrap();
                    format!("{took:.0} µs")
                } else {
                    "stack overflow".to_owned()
                };
                format!(
                    "{}: {time}",
                    if *engine == "nana" {
                        "NanaUI (whole frame)"
                    } else {
                        "Taffy (layout)"
                    }
                )
            })
            .collect();
        println!(
            "| deep tree: a chain of {depth} flex columns on a 512 KiB thread | {} |",
            line.join("; ")
        );
    }
}

fn features(rows: usize) {
    let rounds = rounds();
    println!(
        "What Taffy has no counterpart for, NanaUI alone. Least of {rounds} rounds ({}).\n",
        if cfg!(debug_assertions) {
            "debug"
        } else {
            "release"
        }
    );
    println!("| scene | cost | incremental |");
    println!("| --- | --- | --- |");
    inline_text(rows, rounds);
    sticky_fixed(rows, rounds);
    moves(rows, rounds);
    hit_and_accessibility(rows, rounds);
    deep_trees(rounds);
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let arg = |i: usize| args.get(i).map(String::as_str);
    let list = |i: usize, default: &[usize]| -> Vec<usize> {
        match arg(i) {
            Some(list) => list.split(',').map(|n| n.parse().unwrap()).collect(),
            None => default.to_vec(),
        }
    };
    match arg(0) {
        Some("compare") | None => {
            let rows = list(1, &[1000]);
            let kinds: Vec<Kind> = match arg(2) {
                Some(list) => list.split(',').map(Kind::parse).collect(),
                None => Kind::ALL.to_vec(),
            };
            compare(&rows, &kinds);
        }
        Some("features") => features(list(1, &[1000])[0]),
        Some("inline") => inline_text(list(1, &[1000])[0], rounds()),
        Some("check") => check(list(1, &[100])[0]),
        Some("probe") => {
            let engine = args[1].clone();
            let depth: usize = args[2].parse().unwrap();
            let stack: usize = args[3].parse().unwrap();
            std::thread::Builder::new()
                .stack_size(stack)
                .spawn(move || deep(&engine, depth))
                .unwrap()
                .join()
                .unwrap();
        }
        Some("reps") => {
            // A fixed amount of work for `/usr/bin/time -l`:
            // `reps <flex|grid> <nana|taffy> <rows> <runs>`.
            let kind = Kind::parse(arg(1).unwrap());
            let taffy = arg(2) == Some("taffy");
            let spec = spec(arg(3).unwrap().parse().unwrap(), 10);
            for _ in 0..arg(4).unwrap().parse::<usize>().unwrap() {
                if taffy {
                    taffy_cold(kind, &spec);
                } else {
                    nana_cold(kind, &spec);
                }
            }
        }
        Some(other) => panic!("unknown command {other}"),
    }
}
