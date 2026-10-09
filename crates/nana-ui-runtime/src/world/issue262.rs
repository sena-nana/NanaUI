//! Issue #262: a virtual list moves by its rows' metric deltas, never by
//! its logical collection.
//!
//! A feed of a million rows sits in a ScrollView forty rows tall, with
//! eight rows of overscan on each side. Its rows size to their content
//! ([`VirtualListItems::measured`]): the list mounts its window, reads each
//! mounted row's height back after a layout pass, and keeps the row index
//! ([`VirtualListLayout`]) the consumer owns. The consumer owns the data.
//!
//! - Gate A: a visible row grows by 8 px. The list looks up no logical row
//!   (the rows sit at the indices they were placed at while the data holds),
//!   takes one row's new height, writes a chunk-local, logarithmic slice of
//!   its index, and moves the rows below; the window stays the window.
//! - Gate B: a cell's text changes and its row keeps its height: the list's
//!   extent and every other row's placement stay, and the list does not lay
//!   out.
//! - Gate C: the window's 21st row grows: the twenty above keep their
//!   placement, the rows below shift their origin, and no row below measures
//!   again -- the same layout work as a row near the window's top.
//! - Gate D: an unmounted row's data changes: no layout work, whether the
//!   consumer says the data moved or not; scrolling there shows the new data.
//! - Gate E: a node in a million-row tree expands with ten children: only
//!   those ten are created, no mounted row is recreated, and the window is
//!   all the tree the list looks up.
//!
//! The layout guard checks every retained pass against a full layout; the
//! world holds only the window, so it stays on at a million rows.

use std::cell::Cell;
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::Arc;

use nana_ui_core::{LayoutStyle, LengthSpec, VirtualTreeLayout, VirtualTreeRow, WorkCounters};

use super::reflow_oracle::{bundled_face_shaper, product_frame};
use super::{DocumentId, StableNodeId};
use crate::{
    AppContext, Entity, LayoutBox, LayoutViewport, List, NanaTextEngineShaper, NodeStyle,
    ScrollAxes, ScrollOffset, ScrollView, Text, VirtualListItems, VirtualListLayout,
    VirtualTreeItems,
};

pub(super) const ROW: f32 = 24.0;
const VISIBLE: usize = 40;
const OVERSCAN: usize = 8;
/// The most rows a window can hold: the visible rows, the overscan on each
/// side, and a row cut by each edge.
const WINDOW: usize = VISIBLE + 2 * OVERSCAN + 2;

fn document() -> DocumentId {
    DocumentId::new(1).unwrap()
}

pub(super) fn viewport() -> LayoutViewport {
    LayoutViewport::new(320.0, VISIBLE as f32 * ROW)
}

/// A ScrollView the size of the viewport holding an empty List.
pub(super) fn port(cx: &mut AppContext) -> (Entity<ScrollView>, Entity<List>) {
    let scroll = cx
        .create_component(document(), ScrollView::new(ScrollAxes::Vertical))
        .unwrap();
    cx.update_component(scroll, |scroll, _| {
        let layout = Arc::make_mut(&mut scroll.style.layout);
        layout.width = Some(LengthSpec::Px(320.0));
        layout.height = Some(LengthSpec::Px(VISIBLE as f32 * ROW));
    })
    .unwrap();
    // The list spans the port, as a feed does: its rows are as wide.
    let list = cx
        .create_component(
            document(),
            List::new().style(NodeStyle {
                layout: Arc::new(LayoutStyle {
                    width: Some(LengthSpec::Percent(100.0)),
                    ..LayoutStyle::default()
                }),
                ..NodeStyle::default()
            }),
        )
        .unwrap();
    cx.append_child(scroll, list).unwrap();
    (scroll, list)
}

fn sized(height: f32) -> NodeStyle {
    NodeStyle {
        layout: Arc::new(LayoutStyle {
            height: Some(LengthSpec::Px(height)),
            ..LayoutStyle::default()
        }),
        ..NodeStyle::default()
    }
}

/// The consumer's data for a row: what its cell says and how tall its
/// content lays out.
#[derive(Debug, Clone, PartialEq)]
pub(super) struct Row {
    pub(super) value: &'static str,
    pub(super) height: f32,
}

impl Default for Row {
    fn default() -> Self {
        Self {
            value: "99",
            height: ROW,
        }
    }
}

fn row_view(row: Row) -> Text {
    Text::new(row.value).style(sized(row.height))
}

/// What one virtual list pass cost, from [`WorkCounters`].
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub(super) struct Pass {
    pub(super) metric_updates: usize,
    pub(super) index_updates: usize,
    pub(super) remeasured: usize,
    pub(super) repositioned: usize,
    pub(super) scanned: usize,
    pub(super) extent_updates: usize,
    pub(super) materialized: usize,
}

impl Pass {
    fn between(before: &WorkCounters, after: &WorkCounters) -> Self {
        Self {
            metric_updates: after.virtual_row_metric_updates - before.virtual_row_metric_updates,
            index_updates: after.virtual_prefix_index_updates - before.virtual_prefix_index_updates,
            remeasured: after.virtual_rows_remeasured - before.virtual_rows_remeasured,
            repositioned: after.virtual_rows_repositioned - before.virtual_rows_repositioned,
            scanned: after.virtual_logical_rows_scanned - before.virtual_logical_rows_scanned,
            extent_updates: after.virtual_scroll_extent_updates
                - before.virtual_scroll_extent_updates,
            materialized: after.virtual_rows_materialized_from_layout
                - before.virtual_rows_materialized_from_layout,
        }
    }
}

/// A measured feed of `rows` rows.
pub(super) struct Feed {
    pub(super) cx: AppContext,
    shaper: NanaTextEngineShaper,
    scroll: Entity<ScrollView>,
    list: Entity<List>,
    pub(super) items: VirtualListItems<u64, Text>,
    layout: VirtualListLayout,
    rows: usize,
    /// Rows whose data is not the default.
    data: HashMap<u64, Row>,
    /// The list's fingerprint: bumped when keys move between indices.
    fingerprint: u64,
    /// Logical rows the list asked the consumer for.
    pub(super) lookups: Rc<Cell<usize>>,
}

impl Feed {
    pub(super) fn new(rows: usize) -> Self {
        let mut cx = AppContext::new();
        let (scroll, list) = port(&mut cx);
        let mut feed = Self {
            cx,
            shaper: bundled_face_shaper(),
            scroll,
            list,
            items: VirtualListItems::measured(),
            // A million rows estimated alike: a count per chunk.
            layout: VirtualListLayout::uniform(rows, ROW),
            rows,
            data: HashMap::new(),
            fingerprint: 0,
            lookups: Rc::new(Cell::new(0)),
        };
        // The port lays out first: the list's window is its viewport.
        feed.frame();
        feed.settle();
        feed
    }

    pub(super) fn frame(&mut self) -> WorkCounters {
        product_frame(&mut self.cx, document(), viewport(), &mut self.shaper)
    }

    /// One sync of the list, as a frame hook runs it: what it cost.
    pub(super) fn sync(&mut self) -> Pass {
        let before = self.cx.last_work_counters();
        let Self {
            cx,
            scroll,
            list,
            items,
            layout,
            rows,
            data,
            fingerprint,
            lookups,
            ..
        } = self;
        let rows = *rows;
        let (at, of) = (Rc::clone(lookups), Rc::clone(lookups));
        cx.sync_virtual_list_measured_with(
            *scroll,
            *list,
            items,
            layout,
            OVERSCAN as f32 * ROW,
            *fingerprint,
            &[],
            move |index| {
                at.set(at.get() + 1);
                index as u64
            },
            move |key| {
                of.set(of.get() + 1);
                usize::try_from(*key).ok().filter(|index| *index < rows)
            },
            |_, key| row_view(data.get(key).cloned().unwrap_or_default()),
            |_, _, _, _| Ok(()),
        )
        .unwrap();
        Pass::between(&before, &self.cx.last_work_counters())
    }

    /// Sync and lay out until a sync commits nothing.
    pub(super) fn settle(&mut self) {
        for _ in 0..8 {
            let generation = self.cx.world().generation();
            self.sync();
            if self.cx.world().generation() == generation {
                return;
            }
            self.frame();
        }
        panic!("the feed keeps moving");
    }

    pub(super) fn scroll_to_row(&mut self, index: usize) {
        self.cx
            .scroll_to(
                self.scroll,
                ScrollOffset {
                    x: 0.0,
                    y: index as f32 * ROW,
                },
            )
            .unwrap();
        self.settle();
    }

    pub(super) fn mounted(&self) -> Vec<u64> {
        self.items.mounted_keys().to_vec()
    }

    fn entity(&self, key: u64) -> Entity<Text> {
        self.items.entity(&key).expect("a mounted row")
    }

    /// The consumer's data for `key` changes. A mounted row shows it at
    /// once; its key stays at its index, so the fingerprint stays.
    pub(super) fn edit(&mut self, key: u64, row: Row) {
        self.data.insert(key, row.clone());
        if let Some(entity) = self.items.entity(&key) {
            self.cx
                .update_component(entity, |text, _| *text = row_view(row))
                .unwrap();
        }
    }

    /// The box of each mounted row's content, in window order.
    pub(super) fn boxes(&self) -> Vec<LayoutBox> {
        self.mounted()
            .into_iter()
            .map(|key| {
                self.cx
                    .world()
                    .layout_box(self.entity(key).stable_id())
                    .unwrap()
            })
            .collect()
    }

    fn node_count(&self) -> usize {
        self.cx.world().document_order(document()).len()
    }
}

/// A visible row grows by 8 px: a sync that reads it back and moves the rows
/// below, and the frames around it.
pub(super) struct Growth {
    pub(super) laid: WorkCounters,
    pub(super) pass: Pass,
    pub(super) placed: WorkCounters,
    pub(super) lookups: usize,
}

/// Grow the row at `at` in the window by 8 px.
pub(super) fn grow(feed: &mut Feed, at: usize) -> Growth {
    let key = feed.mounted()[at];
    feed.edit(
        key,
        Row {
            height: ROW + 8.0,
            ..Row::default()
        },
    );
    let lookups = feed.lookups.get();
    let laid = feed.frame();
    let pass = feed.sync();
    let placed = feed.frame();
    let lookups = feed.lookups.get() - lookups;
    feed.settle();
    Growth {
        laid,
        pass,
        placed,
        lookups,
    }
}

#[test]
fn issue262_gate_a_a_row_growing_in_a_million_scans_its_window_at_most() {
    let mut feed = Feed::new(1_000_000);
    feed.scroll_to_row(500_000);
    let window = feed.mounted();
    assert!(
        window.len() <= WINDOW,
        "{} rows mounted for a window of {WINDOW}",
        window.len()
    );
    // The retained tree is the window: a placement container and a cell per
    // row, the port, the list, and the port's own chrome.
    let nodes = feed.node_count();
    assert!(nodes <= 2 * WINDOW + 8, "{nodes} nodes for {WINDOW} rows");

    let at = 10;
    let before = feed.boxes();
    let growth = grow(&mut feed, at);
    let after = feed.boxes();

    assert_eq!(feed.mounted(), window, "the window is the window");
    assert!(
        growth.pass.scanned <= 64 && growth.lookups <= 64,
        "{:?}, {} lookups",
        growth.pass,
        growth.lookups
    );
    assert_eq!(growth.pass.remeasured, 1, "{:?}", growth.pass);
    assert_eq!(growth.pass.metric_updates, 1, "{:?}", growth.pass);
    // The grown row's chunk stops being one extent (its 512 rows, once),
    // then O(log C) index entries over C chunks: never a million.
    assert!(growth.pass.index_updates <= 512 + 32, "{:?}", growth.pass);
    assert_eq!(growth.pass.repositioned, window.len() - at - 1);
    assert_eq!(growth.pass.extent_updates, 1);
    assert_eq!(growth.pass.materialized, 0);
    // The row lays out at its new height: its cell and its row measure.
    assert!(
        growth.laid.layout_measure_nodes <= 2,
        "{}",
        growth.laid.layout_measure_nodes
    );
    // The rows below shift their origin; none of them measures. What the
    // pass measures is the list, whose height moved.
    assert!(
        growth.placed.layout_measure_nodes <= 2,
        "{}",
        growth.placed.layout_measure_nodes
    );
    assert_eq!(growth.placed.intrinsic_measure_full_subtrees, 0);
    assert!(growth.placed.layout_origin_only_updates >= growth.pass.repositioned);
    assert_eq!(growth.placed.layout_full_document_fallbacks, 0);
    for (index, (before, after)) in before.iter().zip(&after).enumerate() {
        let (moved, grew) = match index.cmp(&at) {
            std::cmp::Ordering::Less => (0.0, 0.0),
            std::cmp::Ordering::Equal => (0.0, 8.0),
            std::cmp::Ordering::Greater => (8.0, 0.0),
        };
        assert_eq!(after.y - before.y, moved, "row {index} moved");
        assert_eq!(after.height - before.height, grew, "row {index} grew");
    }

    // A second row in the same chunk writes only index entries.
    let again = grow(&mut feed, at + 1);
    assert!(again.pass.index_updates <= 32, "{:?}", again.pass);
}

#[test]
fn issue262_gate_b_a_cell_that_keeps_its_height_moves_nothing() {
    let mut feed = Feed::new(1_000_000);
    feed.scroll_to_row(500_000);
    let key = feed.mounted()[10];
    let before = feed.boxes();
    feed.edit(
        key,
        Row {
            value: "98",
            ..Row::default()
        },
    );
    let laid = feed.frame();
    // The cell shapes and measures again; its row and the list do not.
    assert!(
        laid.layout_measure_nodes <= 1,
        "{}",
        laid.layout_measure_nodes
    );
    let generation = feed.cx.world().generation();
    let pass = feed.sync();
    assert_eq!(pass, Pass::default(), "the list has nothing to move");
    assert_eq!(feed.cx.world().generation(), generation, "nothing commits");
    let passes = feed.cx.layout_invocations();
    feed.frame();
    assert_eq!(feed.cx.layout_invocations(), passes, "nothing lays out");
    assert_eq!(feed.boxes(), before);
}

#[test]
fn issue262_gate_c_rows_above_a_grown_row_stay_and_rows_below_shift() {
    let mut feed = Feed::new(1_000_000);
    feed.scroll_to_row(500_000);
    let window = feed.mounted().len();
    let containers = |feed: &Feed| {
        feed.mounted()
            .into_iter()
            .map(|key| {
                let id = feed.entity(key).stable_id();
                let container = feed.cx.world().parent_id(id).unwrap();
                feed.cx
                    .world()
                    .node_style(container)
                    .unwrap()
                    .layout
                    .clone()
            })
            .collect::<Vec<_>>()
    };
    let placements = containers(&feed);
    let before = feed.boxes();
    let late = grow(&mut feed, 20);
    let after = feed.boxes();
    let placed = containers(&feed);

    // Rows 0..19 of the window: their placement is the one they had.
    for index in 0..20 {
        assert!(
            Arc::ptr_eq(&placements[index], &placed[index]),
            "row {index} was placed again"
        );
        assert_eq!(after[index], before[index]);
    }
    // Rows after 20 shift their origin and keep their size.
    assert_eq!(late.pass.repositioned, window - 21);
    for index in 21..window {
        assert_eq!(after[index].y - before[index].y, 8.0, "row {index}");
        assert_eq!(after[index].height, before[index].height, "row {index}");
    }

    // A row near the window's top moves more rows the same way, and lays out
    // exactly as much: no row below a grown row measures.
    let mut feed = Feed::new(1_000_000);
    feed.scroll_to_row(500_000);
    let early = grow(&mut feed, 2);
    assert_eq!(early.pass.repositioned, window - 3);
    assert_eq!(
        early.placed.layout_measure_nodes,
        late.placed.layout_measure_nodes
    );
    assert_eq!(
        early.laid.layout_measure_nodes,
        late.laid.layout_measure_nodes
    );
}

#[test]
fn issue262_gate_d_an_unmounted_row_changes_without_layout() {
    let mut feed = Feed::new(1_000_000);
    feed.scroll_to_row(500_000);
    let far = 900_000u64;
    let row = Row {
        value: "98",
        height: 48.0,
    };
    let passes = feed.cx.layout_invocations();

    // The consumer knows the change moved no key: the range gate holds.
    feed.edit(far, row.clone());
    let generation = feed.cx.world().generation();
    let pass = feed.sync();
    assert_eq!(pass, Pass::default());
    assert_eq!(feed.cx.world().generation(), generation);
    feed.frame();
    assert_eq!(feed.cx.layout_invocations(), passes, "no layout work");

    // A consumer that bumps its fingerprint on any change: the mounted rows
    // are looked up again to be measured, and the window is keyed again --
    // twice the window, never the collection -- and nothing commits.
    feed.fingerprint += 1;
    let pass = feed.sync();
    assert!(pass.scanned <= 2 * WINDOW, "{pass:?}");
    assert_eq!(Pass { scanned: 0, ..pass }, Pass::default());
    assert_eq!(feed.cx.world().generation(), generation);
    feed.frame();
    assert_eq!(feed.cx.layout_invocations(), passes, "no layout work");

    // Scrolled into the window, the row shows the data as it is now.
    feed.scroll_to_row(far as usize);
    let entity = feed.entity(far);
    assert_eq!(
        feed.cx.read(entity, |text| text.value.clone()).unwrap(),
        "98"
    );
    let at = feed.mounted().iter().position(|key| *key == far).unwrap();
    assert_eq!(feed.boxes()[at].height, 48.0);
}

#[test]
fn issue262_gate_e_expanding_a_node_in_a_million_creates_only_its_children() {
    const NODES: usize = 1_000_000;
    const CHILDREN: usize = 10;
    let mut cx = AppContext::new();
    let mut shaper = bundled_face_shaper();
    let (scroll, list) = port(&mut cx);
    let mut tree = VirtualTreeLayout::uniform(ROW, std::iter::repeat_n(0, NODES));
    let mut items = VirtualTreeItems::<u64, Text>::default();
    // The node at `parent` holds its children after it once expanded. Keys
    // are node ids; children are numbered from 2,000,000.
    let expanded: Cell<Option<usize>> = Cell::new(None);
    let lookups = Cell::new(0usize);
    let key_at = |index: usize| -> u64 {
        lookups.set(lookups.get() + 1);
        match expanded.get() {
            Some(parent) if index > parent && index <= parent + CHILDREN => {
                2_000_000 + (index - parent - 1) as u64
            }
            Some(parent) if index > parent + CHILDREN => (index - CHILDREN) as u64,
            _ => index as u64,
        }
    };
    let index_of_key = |key: &u64| -> Option<usize> {
        lookups.set(lookups.get() + 1);
        let key = usize::try_from(*key).ok()?;
        match expanded.get() {
            Some(parent) if key >= 2_000_000 => Some(parent + 1 + key - 2_000_000),
            Some(parent) if key > parent => Some(key + CHILDREN),
            _ => (key < NODES).then_some(key),
        }
    };
    let mut fingerprint = 0;
    let sync = |cx: &mut AppContext,
                items: &mut VirtualTreeItems<u64, Text>,
                tree: &VirtualTreeLayout,
                fingerprint: u64| {
        let before = cx.last_work_counters();
        cx.sync_virtual_tree_retained_in(
            scroll,
            list,
            items,
            tree,
            OVERSCAN as f32 * ROW,
            fingerprint,
            &[],
            key_at,
            index_of_key,
            |_, key| Text::new(format!("node {key}")),
        )
        .unwrap();
        Pass::between(&before, &cx.last_work_counters())
    };
    let frame = |cx: &mut AppContext, shaper: &mut NanaTextEngineShaper| -> WorkCounters {
        product_frame(cx, document(), viewport(), shaper)
    };
    frame(&mut cx, &mut shaper);
    sync(&mut cx, &mut items, &tree, fingerprint);
    frame(&mut cx, &mut shaper);
    cx.scroll_to(
        scroll,
        ScrollOffset {
            x: 0.0,
            y: 500_000.0 * ROW,
        },
    )
    .unwrap();
    sync(&mut cx, &mut items, &tree, fingerprint);
    frame(&mut cx, &mut shaper);

    let mounted = items.mounted_keys().to_vec();
    let entities: HashMap<u64, StableNodeId> = mounted
        .iter()
        .map(|key| (*key, items.entity(key).unwrap().stable_id()))
        .collect();
    let parent = 500_010;
    assert!(mounted.contains(&(parent as u64)));
    let lookups_before = lookups.get();

    assert!(tree.expand(
        parent,
        std::iter::repeat_n(
            VirtualTreeRow {
                extent: ROW,
                descendant_count: 0,
            },
            CHILDREN,
        ),
    ));
    expanded.set(Some(parent));
    fingerprint += 1;
    let pass = sync(&mut cx, &mut items, &tree, fingerprint);
    let laid = frame(&mut cx, &mut shaper);

    let now = items.mounted_keys().to_vec();
    assert_eq!(now.len(), mounted.len(), "the window holds as many rows");
    let created: Vec<u64> = now
        .iter()
        .copied()
        .filter(|key| !entities.contains_key(key))
        .collect();
    assert_eq!(
        created,
        (0..CHILDREN as u64)
            .map(|child| 2_000_000 + child)
            .collect::<Vec<_>>(),
        "only the children are created"
    );
    assert_eq!(pass.materialized, CHILDREN, "{pass:?}");
    // Every row that stayed is the row it was: no lifecycle churn.
    for key in now.iter().filter(|key| entities.contains_key(key)) {
        assert_eq!(
            items.entity(key).unwrap().stable_id(),
            entities[key],
            "row {key} was recreated"
        );
    }
    // The window is all of the tree the list looked up.
    let looked_up = lookups.get() - lookups_before;
    assert!(
        pass.scanned <= WINDOW && looked_up <= WINDOW,
        "{pass:?}, {looked_up} lookups"
    );
    // Ten rows join one chunk of a count: O(log C) index entries.
    assert!(pass.index_updates <= 32, "{pass:?}");
    assert_eq!(pass.extent_updates, 1);
    // The children lay out in full, once each, and nothing else does: the
    // rows below the parent move by their origin, at a height of their own.
    assert_eq!(laid.intrinsic_measure_full_subtrees, CHILDREN);
    assert!(
        laid.layout_origin_only_updates >= pass.repositioned,
        "{} origin-only updates for {pass:?}",
        laid.layout_origin_only_updates
    );
    assert_eq!(laid.layout_full_document_fallbacks, 0);
}

/// A table's cell changes what it says and keeps its size: the edit stays in
/// the cell. Its row, the other cells and the table lay nothing out, and the
/// table's window is not placed again.
#[test]
fn issue262_a_table_cell_edit_stays_in_its_cell() {
    use crate::{Table, TableCell, TableRow, VirtualTableItems};
    use nana_ui_core::{TableColumn, VirtualTableLayout, VirtualViewport};

    let mut cx = AppContext::new();
    let mut shaper = bundled_face_shaper();
    let table = cx.create_component(document(), Table::new()).unwrap();
    let layout = VirtualTableLayout::new(
        std::iter::repeat_n(ROW, 1_000_000),
        (0..1_000).map(|column| TableColumn::new(column.to_string(), 80.0)),
    );
    let mut items = VirtualTableItems::<usize, usize>::default();
    let window = VirtualViewport {
        offset: [0.0, 500_000.0 * ROW],
        extent: [320.0, VISIBLE as f32 * ROW],
        overscan: [0.0; 2],
    };
    let fill = |cx: &mut AppContext, items: &mut VirtualTableItems<usize, usize>| {
        cx.materialize_virtual_table_retained_in(
            table,
            items,
            &layout,
            window,
            [0, 0],
            &[],
            |index| index,
            |key| Some(*key),
            |index| index,
            |key| Some(*key),
            |_, _| TableRow::new(),
            |_, _, _, _| TableCell::new("99"),
        )
        .unwrap();
    };
    fill(&mut cx, &mut items);
    product_frame(&mut cx, document(), viewport(), &mut shaper);
    let order = cx.world().document_order(document());
    let boxes = |cx: &AppContext| {
        order
            .iter()
            .map(|id| cx.world().layout_box(*id))
            .collect::<Vec<_>>()
    };
    let before = boxes(&cx);
    let cell = items.cell_entity(&500_010, &2).unwrap();
    cx.update_component(cell, |cell, _| cell.value = "98".into())
        .unwrap();
    let laid = product_frame(&mut cx, document(), viewport(), &mut shaper);
    // The cell shaped its new text in the box it has; nothing lays out.
    assert!(laid.text_shaped >= 1, "the cell shaped again");
    assert_eq!(laid.layout_frontier_seeds, 0);
    assert_eq!(laid.layout_measure_nodes, 0);
    assert_eq!(laid.layout_placement_nodes, 0);
    assert_eq!(boxes(&cx), before);
    let generation = cx.world().generation();
    fill(&mut cx, &mut items);
    assert_eq!(
        cx.world().generation(),
        generation,
        "the window stays placed"
    );
}
