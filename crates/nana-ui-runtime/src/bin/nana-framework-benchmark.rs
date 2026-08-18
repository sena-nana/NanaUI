use std::sync::Arc;
use std::time::{Duration, Instant};

use nana_ui_core::{LengthSpec, TableColumn, VirtualListLayout, VirtualTableLayout};
use nana_ui_runtime::{
    Activate, AppContext, Button, ContextPredicate, Dialog, Dock, DockAxis, DockNode, DocumentId,
    KeyContext, LayoutViewport, List, MeasureTextShaper, Menu, NodeKind, NodeStyle, OverlayHost,
    Popover, ScrollAxes, ScrollOffset, ScrollView, StableNodeId, SystemWork, Table, TableCell,
    TableRow, Text, TextArea, TextContent, TextInput, Tooltip, VirtualListItems, VirtualTableItems,
    WorkCounters,
};
use serde::Serialize;

const WARMUP_ITERATIONS: usize = 100;
const ITERATIONS: usize = 1_000;
const FRAME_BUDGET_MS: f64 = 16.67;
const SCALE_100K_WARMUP: usize = 10;
const SCALE_100K_ITERATIONS: usize = 40;
const SCALE_1M_WARMUP: usize = 5;
const SCALE_1M_ITERATIONS: usize = 15;
const WORKLOAD_WARMUP: usize = 10;
const WORKLOAD_ITERATIONS: usize = 40;
const LARGE_SCALE_TIMEOUT: Duration = Duration::from_secs(90);
const IME_SCRIPTS: [(&str, &str, &str); 4] = [
    ("latin", "hello", "hello"),
    ("zh", "nihao", "你好"),
    ("ja", "nihongo", "日本語"),
    ("ko", "hangug", "한글"),
];
const OVERLAY_KINDS: [&str; 4] = ["tooltip", "context_menu", "modal", "popup"];
const TEXT_EDITOR_CHARS: usize = 100_000;
const TEXT_EDITOR_VISIBLE_LINES: usize = 40;
const TEXT_EDITOR_LINE_PX: f32 = 20.0;
const DOCK_PANES: usize = 8;
const DOCK_VIEWPORT: (f32, f32) = (1_280.0, 800.0);
const LIST_VIEWPORT: f32 = 800.0;
const LIST_OVERSCAN: f32 = 200.0;
const LIST_ITEM_EXTENT: f32 = 20.0;
const TABLE_VIEWPORT: (f32, f32) = (1_280.0, 800.0);
const TABLE_OVERSCAN: (f32, f32) = (160.0, 200.0);
const TABLE_COLUMN_EXTENT: f32 = 80.0;
/// `perf/scenarios/text-table.json` / catalog params. Most cells are short;
/// each 40-row band keeps `WRAPPED_CELLS` long wrapping cells in column 0.
const SHORT_CELL_LEN: usize = 8;
const WRAPPED_CELLS: usize = 4;
const WRAPPED_CELL_LEN: usize = 256;
const TEXT_TABLE_VISIBLE_ROWS: usize = 40;

/// Independent of `materialized.range`. Two-sided overscan plus one partial
/// item on each edge. For 800+2×200 / 20 this is 62 list rows (~60).
fn list_live_entity_bound() -> usize {
    VirtualListLayout::uniform_window_item_cap(LIST_VIEWPORT, LIST_OVERSCAN, LIST_ITEM_EXTENT)
}

fn table_column_cap() -> usize {
    VirtualListLayout::uniform_window_item_cap(
        TABLE_VIEWPORT.0,
        TABLE_OVERSCAN.0,
        TABLE_COLUMN_EXTENT,
    )
}

fn table_row_cap() -> usize {
    VirtualListLayout::uniform_window_item_cap(TABLE_VIEWPORT.1, TABLE_OVERSCAN.1, LIST_ITEM_EXTENT)
}

fn table_live_entity_bound() -> usize {
    let rows = table_row_cap();
    rows + rows * table_column_cap()
}

/// Four wrapping cells per contract-visible band (column 0 of the first four rows).
fn is_wrapped_cell(row: usize, column: usize) -> bool {
    column == 0 && row % TEXT_TABLE_VISIBLE_ROWS < WRAPPED_CELLS
}

fn padded_cell_text(prefix: &str, row: usize, column: usize, len: usize) -> String {
    let mut text = format!("{prefix}{row}:{column}");
    if text.len() < len {
        text.extend(std::iter::repeat_n('x', len - text.len()));
    }
    text.truncate(len);
    text
}

fn text_table_cell(row: usize, column: usize) -> TableCell {
    if is_wrapped_cell(row, column) {
        TableCell::new(padded_cell_text("wrap ", row, column, WRAPPED_CELL_LEN))
            .style(wrapped_cell_style())
    } else {
        TableCell::new(padded_cell_text("", row, column, SHORT_CELL_LEN))
    }
}

fn wrapped_cell_style() -> NodeStyle {
    NodeStyle {
        layout: Arc::new(nana_ui_core::LayoutStyle {
            width: Some(LengthSpec::Px(TABLE_COLUMN_EXTENT)),
            max_width: Some(LengthSpec::Px(TABLE_COLUMN_EXTENT)),
            white_space_nowrap: false,
            padding_left: Some(LengthSpec::Px(nana_ui_core::UI_METRICS.list_item_padding_x)),
            padding_right: Some(LengthSpec::Px(nana_ui_core::UI_METRICS.list_item_padding_x)),
            ..nana_ui_core::LayoutStyle::default()
        }),
        ..NodeStyle::default()
    }
}

/// Shape + wrap-measure + extract after table materialize so `text_shaped` /
/// `extracted_text_spans` / `text_wrap_layouts` observe wrapping cells.
fn measure_virtual_table_text(context: &mut AppContext, document: DocumentId, work: &SystemWork) {
    let mut shaper = MeasureTextShaper;
    let _ = context.resolve_styles(&work.style);
    let _ = context.shape_text(&work.text, &mut shaper);
    let _ = context.layout_document(
        document,
        LayoutViewport::new(TABLE_VIEWPORT.0, TABLE_VIEWPORT.1),
    );
    let _ = context.shape_text_for_layout(document, &mut shaper);
    let extracted = context.world().extract_nodes(&work.render_extraction);
    context.record_extract(&extracted);
}

/// Own AppContext so table shaping cannot dirty the shared list/scroll drain.
fn isolated_table_text_work(logical_rows: usize, logical_columns: usize) -> ScaleWork {
    let document = DocumentId::new(1).unwrap();
    let mut context = AppContext::new();
    let table = context.create_component(document, Table::new()).unwrap();
    let mut items = VirtualTableItems::<usize, usize>::default();
    let layout = VirtualTableLayout::new(
        std::iter::repeat_n(20.0, logical_rows),
        (0..logical_columns).map(|index| TableColumn::new(format!("column-{index}"), 80.0)),
    );
    let _ = context.take_system_work();
    context
        .materialize_virtual_table(
            table,
            &mut items,
            &layout,
            (0.0, 0.0),
            TABLE_VIEWPORT,
            TABLE_OVERSCAN,
            |index| index,
            |index| index,
            |_index, _| TableRow::new(),
            |row, _, column, _| text_table_cell(row, column),
        )
        .unwrap();
    let work = context.take_system_work();
    measure_virtual_table_text(&mut context, document, &work);
    ScaleWork::from(context.last_work_counters())
}

#[derive(Default)]
struct Counter(usize);

struct Increment;

#[derive(Serialize)]
struct Report {
    schema_version: u32,
    profile: &'static str,
    frame_budget_ms: f64,
    frame_budget_hz: u32,
    warmup_iterations: usize,
    iterations: usize,
    view_event_update_ms: Distribution,
    action_dispatch_ms: Distribution,
    component_activate_ms: Distribution,
    component_noop_ms: Distribution,
    virtual_list_10k_window_ms: Distribution,
    virtual_list_10k_update_ms: Distribution,
    virtual_list_10k_materialize_ms: Distribution,
    virtual_table_10k_x_100_window_ms: Distribution,
    virtual_table_10k_x_100_materialize_ms: Distribution,
    virtual_table_column_resize_ms: Distribution,
    virtual_scroll_40_visible_nodes_ms: Distribution,
    canonical_layout_5000_nodes_ms: Distribution,
    virtual_scales: Vec<VirtualScaleCase>,
    catalog_workloads: Vec<CatalogWorkloadCase>,
    virtual_tree: SkippedApi,
}

#[derive(Serialize)]
struct VirtualScaleCase {
    kind: &'static str,
    logical_rows: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    logical_columns: Option<usize>,
    status: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    skip_reason: Option<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    visible_rows: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    overscan_rows: Option<usize>,
    cache_rows: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    live_ui_entities: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    live_ui_entities_bound: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    construction_ms: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    window_ms: Option<Distribution>,
    #[serde(skip_serializing_if = "Option::is_none")]
    materialize_ms: Option<Distribution>,
    /// Existing WorkCounters from the last table/list drain. Glyph cache
    /// keys stay omitted: Runtime does not observe them.
    #[serde(skip_serializing_if = "Option::is_none")]
    work: Option<ScaleWork>,
}

/// Subset of [`WorkCounters`] that this binary already observes.
#[derive(Serialize, Clone, Copy)]
struct ScaleWork {
    entities_total: usize,
    entities_changed: usize,
    entities_spawned: usize,
    entities_despawned: usize,
    style_processed: usize,
    text_shaped: usize,
    layout_nodes: usize,
    hit_test_candidates: usize,
    input_targets: usize,
    accessibility_nodes_updated: usize,
    render_nodes_changed: usize,
    render_nodes_extracted: usize,
    extracted_text_spans: usize,
    text_shaped_runs: usize,
    text_layout_cache_hits: usize,
    text_layout_cache_misses: usize,
    text_wrap_layouts: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    glyph_cache_hits: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    glyph_cache_misses: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    cache_eviction: Option<usize>,
}

/// Issue #8 catalog workloads that reuse existing Runtime APIs (IME, dock,
/// overlay, editor). Glyph cache keys stay omitted.
#[derive(Serialize)]
struct CatalogWorkloadCase {
    id: &'static str,
    kind: &'static str,
    status: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    skip_reason: Option<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    scripts: Option<Vec<&'static str>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    panes: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    overlay_kinds: Option<Vec<&'static str>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    document_chars: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    visible_lines: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    live_ui_entities: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    ime_script_count: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    overlay_kind_count: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    preedit_ms: Option<Distribution>,
    #[serde(skip_serializing_if = "Option::is_none")]
    commit_ms: Option<Distribution>,
    #[serde(skip_serializing_if = "Option::is_none")]
    resize_ms: Option<Distribution>,
    #[serde(skip_serializing_if = "Option::is_none")]
    activate_ms: Option<Distribution>,
    #[serde(skip_serializing_if = "Option::is_none")]
    local_edit_ms: Option<Distribution>,
    #[serde(skip_serializing_if = "Option::is_none")]
    work: Option<ScaleWork>,
}

impl From<WorkCounters> for ScaleWork {
    fn from(counters: WorkCounters) -> Self {
        Self {
            entities_total: counters.entities_total,
            entities_changed: counters.entities_changed,
            entities_spawned: counters.entities_spawned,
            entities_despawned: counters.entities_despawned,
            style_processed: counters.style_processed,
            text_shaped: counters.text_shaped,
            layout_nodes: counters.layout_nodes,
            hit_test_candidates: counters.hit_test_candidates,
            input_targets: counters.input_targets,
            accessibility_nodes_updated: counters.accessibility_nodes_updated,
            render_nodes_changed: counters.render_nodes_changed,
            render_nodes_extracted: counters.render_nodes_extracted,
            extracted_text_spans: counters.extracted_text_spans,
            text_shaped_runs: counters.text_shaped_runs,
            text_layout_cache_hits: counters.text_layout_cache_hits,
            text_layout_cache_misses: counters.text_layout_cache_misses,
            text_wrap_layouts: counters.text_wrap_layouts,
            glyph_cache_hits: counters.glyph_cache_hits,
            glyph_cache_misses: counters.glyph_cache_misses,
            cache_eviction: counters.cache_eviction,
        }
    }
}

#[derive(Serialize)]
struct SkippedApi {
    status: &'static str,
    reason: &'static str,
}

#[derive(Serialize)]
struct Distribution {
    p50: f64,
    p95: f64,
    p99: f64,
    max: f64,
    frame_budget_ms: f64,
    frame_budget_misses: usize,
}

fn main() {
    let mut context = AppContext::new();
    let entity = context
        .create_view(
            DocumentId::new(1).unwrap(),
            NodeKind::Text,
            Counter::default(),
        )
        .unwrap();
    context
        .on(entity, move |view, _event: &Increment, cx| {
            view.0 += 1;
            cx.mutations().set_text(
                entity.stable_id(),
                TextContent {
                    value: view.0.to_string(),
                },
            );
        })
        .unwrap();
    context
        .register_action("benchmark.action", ContextPredicate::always(), |_| Ok(()))
        .unwrap();
    let action = "benchmark.action".into();
    let key_context = KeyContext::default();
    let button = context
        .create_component(DocumentId::new(1).unwrap(), Button::new("Build"))
        .unwrap();
    let scroll = context
        .create_component(
            DocumentId::new(1).unwrap(),
            ScrollView::new(ScrollAxes::Vertical),
        )
        .unwrap();
    let materialized_list = context
        .create_component(DocumentId::new(1).unwrap(), List::new())
        .unwrap();
    let mut materialized_items = VirtualListItems::<usize, Text>::default();
    let materialized_table = context
        .create_component(DocumentId::new(1).unwrap(), Table::new())
        .unwrap();
    let mut materialized_table_items = VirtualTableItems::<usize, usize>::default();
    for index in 0..40 {
        let row = context
            .create_component(
                DocumentId::new(1).unwrap(),
                Text::new(format!("Visible row {index}")),
            )
            .unwrap();
        context.append_child(scroll, row).unwrap();
    }
    context
        .on(button, |button, _event: &Activate, _cx| {
            button.label = if button.label == "Build" {
                "Stop".into()
            } else {
                "Build".into()
            };
        })
        .unwrap();
    let _ = context.take_system_work();
    let mut updates = Vec::with_capacity(ITERATIONS);
    let mut actions = Vec::with_capacity(ITERATIONS);
    let mut component_activations = Vec::with_capacity(ITERATIONS);
    let mut component_noops = Vec::with_capacity(ITERATIONS);
    let mut virtual_list_windows = Vec::with_capacity(ITERATIONS);
    let mut virtual_list_updates = Vec::with_capacity(ITERATIONS);
    let mut virtual_list_materializations = Vec::with_capacity(ITERATIONS);
    let mut virtual_table_windows = Vec::with_capacity(ITERATIONS);
    let mut virtual_table_materializations = Vec::with_capacity(ITERATIONS);
    let mut virtual_table_resizes = Vec::with_capacity(ITERATIONS);
    let mut virtual_scroll_updates = Vec::with_capacity(ITERATIONS);
    let mut canonical_layout_updates = Vec::with_capacity(ITERATIONS);
    let mut virtual_list = VirtualListLayout::new(std::iter::repeat_n(20.0, 10_000));
    let mut virtual_table = VirtualTableLayout::new(
        std::iter::repeat_n(20.0, 10_000),
        (0..100).map(|index| TableColumn::new(format!("column-{index}"), 80.0)),
    );
    let layout_document = DocumentId::new(2).unwrap();
    let mut layout_context = AppContext::new();
    let layout_root = layout_context
        .create_component(layout_document, List::new())
        .unwrap();
    for index in 0..4_999 {
        let child = layout_context
            .create_component(layout_document, Text::new(format!("row {index}")))
            .unwrap();
        layout_context.append_child(layout_root, child).unwrap();
    }
    let _ = layout_context.take_system_work();
    let mut last_list_visible = 0;
    let mut last_list_overscan = 0;
    let mut last_list_live = 0;
    let mut last_table_visible = 0;
    let mut last_table_overscan = 0;
    let mut last_table_live = 0;
    for iteration in 0..(WARMUP_ITERATIONS + ITERATIONS) {
        let started = Instant::now();
        context
            .update(entity, |_view, cx| cx.emit(Increment))
            .unwrap();
        let update_elapsed = started.elapsed();
        let _ = context.take_system_work();

        let started = Instant::now();
        context.dispatch_action(&action, &key_context).unwrap();
        let action_elapsed = started.elapsed();

        let started = Instant::now();
        assert!(context.activate_button(button).unwrap());
        let component_activate_elapsed = started.elapsed();
        let _ = context.take_system_work();
        let generation = context.world().generation();
        let started = Instant::now();
        context.update_component(button, |_button, _cx| {}).unwrap();
        let component_noop_elapsed = started.elapsed();
        assert_eq!(context.world().generation(), generation);
        assert!(context.take_system_work().is_empty());

        let started = Instant::now();
        let window = virtual_list.window((iteration * 13) as f32, LIST_VIEWPORT, LIST_OVERSCAN);
        let virtual_list_window_elapsed = started.elapsed();
        assert!(!window.range.is_empty());
        let item = iteration % 10;
        let extent = if (iteration / 10).is_multiple_of(2) {
            21.0
        } else {
            20.0
        };
        let started = Instant::now();
        let _ = virtual_list.update_item_extent(item, extent);
        let virtual_list_update_elapsed = started.elapsed();
        let materialize_offset = if iteration.is_multiple_of(2) {
            120.0
        } else {
            140.0
        };
        let started = Instant::now();
        let materialized = context
            .materialize_virtual_list(
                materialized_list,
                &mut materialized_items,
                &virtual_list,
                materialize_offset,
                LIST_VIEWPORT,
                LIST_OVERSCAN,
                |index| index,
                |index, _| Text::new(format!("Visible row {index}")),
            )
            .unwrap();
        let virtual_list_materialize_elapsed = started.elapsed();
        let live_list = context
            .world()
            .node(materialized_list.stable_id())
            .unwrap()
            .children
            .len();
        let visible_list = virtual_list
            .window(materialize_offset, LIST_VIEWPORT, 0.0)
            .range
            .len();
        let overscan_list = materialized.range.len().saturating_sub(visible_list);
        let list_bound = list_live_entity_bound();
        assert_eq!(live_list, materialized.range.len());
        assert!(
            live_list <= list_bound,
            "virtual list live entities {live_list} exceed geometric bound {list_bound}"
        );
        if iteration >= WARMUP_ITERATIONS {
            last_list_visible = visible_list;
            last_list_overscan = overscan_list;
            last_list_live = live_list;
        }
        let _ = context.take_system_work();
        let started = Instant::now();
        let table_window = virtual_table.window(
            ((iteration * 37) as f32, (iteration * 131) as f32),
            TABLE_VIEWPORT,
            TABLE_OVERSCAN,
        );
        let virtual_table_window_elapsed = started.elapsed();
        assert!(!table_window.rows.range.is_empty());
        assert!(!table_window.columns.range.is_empty());
        let materialize_scroll = if iteration.is_multiple_of(2) {
            (0.0, 120.0)
        } else {
            (80.0, 140.0)
        };
        let started = Instant::now();
        let materialized_table_window = context
            .materialize_virtual_table(
                materialized_table,
                &mut materialized_table_items,
                &virtual_table,
                materialize_scroll,
                TABLE_VIEWPORT,
                TABLE_OVERSCAN,
                |index| index,
                |index| index,
                |_index, _| TableRow::new(),
                |row, _, column, _| text_table_cell(row, column),
            )
            .unwrap();
        let virtual_table_materialize_elapsed = started.elapsed();
        let mounted_rows = materialized_table_window.rows.range.len();
        let mounted_columns = materialized_table_window.columns.range.len();
        let visible_table = virtual_table.window(materialize_scroll, TABLE_VIEWPORT, (0.0, 0.0));
        let visible_rows = visible_table.rows.range.len();
        let overscan_rows = mounted_rows.saturating_sub(visible_rows);
        let live_table = mounted_rows
            + materialized_table_items
                .mounted_rows()
                .iter()
                .map(|row| {
                    context
                        .world()
                        .node(
                            materialized_table_items
                                .row_entity(row)
                                .unwrap()
                                .stable_id(),
                        )
                        .unwrap()
                        .children
                        .len()
                })
                .sum::<usize>();
        let live_table_bound = table_live_entity_bound();
        assert!(
            mounted_rows <= table_row_cap(),
            "virtual table mounted rows {mounted_rows} exceed geometric row cap {}",
            table_row_cap()
        );
        assert!(
            mounted_columns <= table_column_cap(),
            "virtual table mounted columns {mounted_columns} exceed geometric column cap {}",
            table_column_cap()
        );
        assert_eq!(
            context
                .world()
                .node(materialized_table.stable_id())
                .unwrap()
                .children
                .len(),
            mounted_rows
        );
        assert!(
            live_table <= live_table_bound,
            "virtual table live entities {live_table} exceed geometric bound {live_table_bound}"
        );
        if iteration >= WARMUP_ITERATIONS {
            last_table_visible = visible_rows;
            last_table_overscan = overscan_rows;
            last_table_live = live_table;
        }
        assert_eq!(
            materialized_table_items
                .mounted_rows()
                .iter()
                .map(|row| context
                    .world()
                    .node(
                        materialized_table_items
                            .row_entity(row)
                            .unwrap()
                            .stable_id(),
                    )
                    .unwrap()
                    .children
                    .len())
                .sum::<usize>(),
            mounted_rows * mounted_columns
        );
        let _ = context.take_system_work();
        let column = iteration % 10;
        let column_extent = if (iteration / 10).is_multiple_of(2) {
            81.0
        } else {
            80.0
        };
        let started = Instant::now();
        let _ = virtual_table.resize_column(column, column_extent);
        let virtual_table_resize_elapsed = started.elapsed();
        let scroll_offset = if iteration.is_multiple_of(2) {
            120.0
        } else {
            140.0
        };
        let started = Instant::now();
        assert!(
            context
                .scroll_to(
                    scroll,
                    ScrollOffset {
                        x: 0.0,
                        y: scroll_offset,
                    },
                )
                .unwrap()
        );
        let virtual_scroll_elapsed = started.elapsed();
        let scroll_work = context.take_system_work();
        // Shared list/scroll drain only. Catalog IME/dock/overlay/editor use
        // their own AppContext after this loop (see catalog_workloads).
        assert_eq!(scroll_work.input_hit_test.len(), 41);
        assert_eq!(scroll_work.render_extraction.len(), 41);
        assert!(scroll_work.layout.is_empty());
        let viewport_width = if iteration.is_multiple_of(2) {
            1_280.0
        } else {
            1_024.0
        };
        let started = Instant::now();
        layout_context
            .layout_document(layout_document, LayoutViewport::new(viewport_width, 800.0))
            .unwrap();
        let canonical_layout_elapsed = started.elapsed();
        let _ = layout_context.take_system_work();
        if iteration >= WARMUP_ITERATIONS {
            updates.push(update_elapsed);
            actions.push(action_elapsed);
            component_activations.push(component_activate_elapsed);
            component_noops.push(component_noop_elapsed);
            virtual_list_windows.push(virtual_list_window_elapsed);
            virtual_list_updates.push(virtual_list_update_elapsed);
            virtual_list_materializations.push(virtual_list_materialize_elapsed);
            virtual_table_windows.push(virtual_table_window_elapsed);
            virtual_table_materializations.push(virtual_table_materialize_elapsed);
            virtual_table_resizes.push(virtual_table_resize_elapsed);
            virtual_scroll_updates.push(virtual_scroll_elapsed);
            canonical_layout_updates.push(canonical_layout_elapsed);
        }
    }
    let mut virtual_scales = vec![
        VirtualScaleCase {
            kind: "list",
            logical_rows: 10_000,
            logical_columns: None,
            status: "ok",
            skip_reason: None,
            visible_rows: Some(last_list_visible),
            overscan_rows: Some(last_list_overscan),
            cache_rows: 0,
            live_ui_entities: Some(last_list_live),
            live_ui_entities_bound: Some(list_live_entity_bound()),
            construction_ms: None,
            window_ms: Some(summarize(&virtual_list_windows)),
            materialize_ms: Some(summarize(&virtual_list_materializations)),
            work: None,
        },
        VirtualScaleCase {
            kind: "table",
            logical_rows: 10_000,
            logical_columns: Some(100),
            status: "ok",
            skip_reason: None,
            visible_rows: Some(last_table_visible),
            overscan_rows: Some(last_table_overscan),
            cache_rows: 0,
            live_ui_entities: Some(last_table_live),
            live_ui_entities_bound: Some(table_live_entity_bound()),
            construction_ms: None,
            window_ms: Some(summarize(&virtual_table_windows)),
            materialize_ms: Some(summarize(&virtual_table_materializations)),
            work: Some(isolated_table_text_work(10_000, 100)),
        },
        bench_virtual_list_scale(100_000, SCALE_100K_WARMUP, SCALE_100K_ITERATIONS, None),
        bench_virtual_table_scale(100_000, 100, SCALE_100K_WARMUP, SCALE_100K_ITERATIONS, None),
    ];
    if large_scale_enabled() {
        virtual_scales.push(bench_virtual_list_scale(
            1_000_000,
            SCALE_1M_WARMUP,
            SCALE_1M_ITERATIONS,
            Some(LARGE_SCALE_TIMEOUT),
        ));
        virtual_scales.push(bench_virtual_table_scale(
            1_000_000,
            100,
            SCALE_1M_WARMUP,
            SCALE_1M_ITERATIONS,
            Some(LARGE_SCALE_TIMEOUT),
        ));
    } else {
        virtual_scales.push(skipped_scale(
            "list",
            1_000_000,
            None,
            "NANA_PERF_SCALE!=large",
        ));
        virtual_scales.push(skipped_scale(
            "table",
            1_000_000,
            Some(100),
            "NANA_PERF_SCALE!=large",
        ));
    }
    write_report(&Report {
        schema_version: 9,
        profile: if cfg!(debug_assertions) {
            "debug"
        } else {
            "release"
        },
        frame_budget_ms: FRAME_BUDGET_MS,
        frame_budget_hz: 60,
        warmup_iterations: WARMUP_ITERATIONS,
        iterations: ITERATIONS,
        view_event_update_ms: summarize(&updates),
        action_dispatch_ms: summarize(&actions),
        component_activate_ms: summarize(&component_activations),
        component_noop_ms: summarize(&component_noops),
        virtual_list_10k_window_ms: summarize(&virtual_list_windows),
        virtual_list_10k_update_ms: summarize(&virtual_list_updates),
        virtual_list_10k_materialize_ms: summarize(&virtual_list_materializations),
        virtual_table_10k_x_100_window_ms: summarize(&virtual_table_windows),
        virtual_table_10k_x_100_materialize_ms: summarize(&virtual_table_materializations),
        virtual_table_column_resize_ms: summarize(&virtual_table_resizes),
        virtual_scroll_40_visible_nodes_ms: summarize(&virtual_scroll_updates),
        canonical_layout_5000_nodes_ms: summarize(&canonical_layout_updates),
        virtual_scales,
        catalog_workloads: vec![
            // Each helper constructs AppContext::new(). Do not fold these
            // into the shared list/table loop — table shaping on that
            // context previously inflated scroll input_hit_test from 41 to 1164.
            bench_ime(),
            bench_dock_workspace(),
            bench_overlay(),
            bench_text_editor(),
        ],
        virtual_tree: SkippedApi {
            status: "skipped",
            reason: "no scale bench this round",
        },
    });
}

fn write_report(report: &Report) {
    let json = serde_json::to_string_pretty(report).expect("benchmark report must serialize");
    let mut arguments = std::env::args_os().skip(1);
    match arguments.next() {
        None => println!("{json}"),
        Some(flag) if flag == "--output" => {
            let path = std::path::PathBuf::from(
                arguments
                    .next()
                    .expect("--output requires a destination path"),
            );
            assert!(arguments.next().is_none(), "unexpected benchmark arguments");
            if let Some(parent) = path
                .parent()
                .filter(|parent| !parent.as_os_str().is_empty())
            {
                std::fs::create_dir_all(parent).expect("benchmark directory must be writable");
            }
            std::fs::write(&path, format!("{json}\n"))
                .expect("benchmark destination must be writable");
            println!("{}", path.display());
        }
        Some(argument) => panic!(
            "unsupported argument `{}`; expected --output <path>",
            argument.to_string_lossy()
        ),
    }
}

fn summarize(samples: &[Duration]) -> Distribution {
    let mut values = samples
        .iter()
        .map(|duration| duration.as_secs_f64() * 1_000.0)
        .collect::<Vec<_>>();
    values.sort_by(f64::total_cmp);
    Distribution {
        p50: percentile(&values, 0.50),
        p95: percentile(&values, 0.95),
        p99: percentile(&values, 0.99),
        max: values
            .last()
            .copied()
            .map(|value| (value * 1_000.0).round() / 1_000.0)
            .unwrap_or(0.0),
        frame_budget_ms: FRAME_BUDGET_MS,
        frame_budget_misses: values
            .iter()
            .filter(|value| **value > FRAME_BUDGET_MS)
            .count(),
    }
}

fn percentile(values: &[f64], percentile: f64) -> f64 {
    let index = ((values.len() - 1) as f64 * percentile).round() as usize;
    (values[index] * 1_000.0).round() / 1_000.0
}

fn large_scale_enabled() -> bool {
    matches!(
        std::env::var("NANA_PERF_SCALE").ok().as_deref(),
        Some("large")
    )
}

fn skipped_scale(
    kind: &'static str,
    logical_rows: usize,
    logical_columns: Option<usize>,
    reason: &'static str,
) -> VirtualScaleCase {
    VirtualScaleCase {
        kind,
        logical_rows,
        logical_columns,
        status: "skipped",
        skip_reason: Some(reason),
        visible_rows: None,
        overscan_rows: None,
        cache_rows: 0,
        live_ui_entities: None,
        live_ui_entities_bound: None,
        construction_ms: None,
        window_ms: None,
        materialize_ms: None,
        work: None,
    }
}

fn bench_virtual_list_scale(
    logical_rows: usize,
    warmup: usize,
    iterations: usize,
    timeout: Option<Duration>,
) -> VirtualScaleCase {
    let started = Instant::now();
    let layout = VirtualListLayout::new(std::iter::repeat_n(20.0, logical_rows));
    let construction_ms = started.elapsed().as_secs_f64() * 1_000.0;
    if timeout.is_some_and(|limit| started.elapsed() > limit) {
        return skipped_scale(
            "list",
            logical_rows,
            None,
            "construction exceeded LARGE_SCALE_TIMEOUT",
        );
    }
    let loop_started = Instant::now();
    let mut context = AppContext::new();
    let list = context
        .create_component(DocumentId::new(1).unwrap(), List::new())
        .unwrap();
    let mut items = VirtualListItems::<usize, Text>::default();
    let _ = context.take_system_work();
    let mut windows = Vec::with_capacity(iterations);
    let mut materializations = Vec::with_capacity(iterations);
    let mut last_visible = 0;
    let mut last_overscan = 0;
    let mut last_live = 0;
    let list_bound = list_live_entity_bound();
    for iteration in 0..(warmup + iterations) {
        if timeout.is_some_and(|limit| loop_started.elapsed() > limit) {
            return skipped_scale(
                "list",
                logical_rows,
                None,
                "materialize loop exceeded LARGE_SCALE_TIMEOUT",
            );
        }
        let scroll = 120.0 + (iteration as f32 * 13.0) % 4_000.0;
        let window_started = Instant::now();
        let window = layout.window(scroll, LIST_VIEWPORT, LIST_OVERSCAN);
        let window_elapsed = window_started.elapsed();
        assert!(!window.range.is_empty());
        let materialize_started = Instant::now();
        let materialized = context
            .materialize_virtual_list(
                list,
                &mut items,
                &layout,
                scroll,
                LIST_VIEWPORT,
                LIST_OVERSCAN,
                |index| index,
                |index, _| Text::new(format!("Visible row {index}")),
            )
            .unwrap();
        let materialize_elapsed = materialize_started.elapsed();
        let live = context
            .world()
            .node(list.stable_id())
            .unwrap()
            .children
            .len();
        let visible = layout.window(scroll, LIST_VIEWPORT, 0.0).range.len();
        let overscan = materialized.range.len().saturating_sub(visible);
        assert_eq!(live, materialized.range.len());
        assert!(
            live <= list_bound,
            "virtual list live entities {live} exceed geometric bound {list_bound}"
        );
        let _ = context.take_system_work();
        if iteration >= warmup {
            windows.push(window_elapsed);
            materializations.push(materialize_elapsed);
            last_visible = visible;
            last_overscan = overscan;
            last_live = live;
        }
    }
    VirtualScaleCase {
        kind: "list",
        logical_rows,
        logical_columns: None,
        status: "ok",
        skip_reason: None,
        visible_rows: Some(last_visible),
        overscan_rows: Some(last_overscan),
        cache_rows: 0,
        live_ui_entities: Some(last_live),
        live_ui_entities_bound: Some(list_bound),
        construction_ms: Some((construction_ms * 1_000.0).round() / 1_000.0),
        window_ms: Some(summarize(&windows)),
        materialize_ms: Some(summarize(&materializations)),
        work: None,
    }
}

fn bench_virtual_table_scale(
    logical_rows: usize,
    logical_columns: usize,
    warmup: usize,
    iterations: usize,
    timeout: Option<Duration>,
) -> VirtualScaleCase {
    let started = Instant::now();
    let layout = VirtualTableLayout::new(
        std::iter::repeat_n(20.0, logical_rows),
        (0..logical_columns).map(|index| TableColumn::new(format!("column-{index}"), 80.0)),
    );
    let construction_ms = started.elapsed().as_secs_f64() * 1_000.0;
    if timeout.is_some_and(|limit| started.elapsed() > limit) {
        return skipped_scale(
            "table",
            logical_rows,
            Some(logical_columns),
            "construction exceeded LARGE_SCALE_TIMEOUT",
        );
    }
    let loop_started = Instant::now();
    let mut context = AppContext::new();
    let table = context
        .create_component(DocumentId::new(1).unwrap(), Table::new())
        .unwrap();
    let mut items = VirtualTableItems::<usize, usize>::default();
    let _ = context.take_system_work();
    let mut windows = Vec::with_capacity(iterations);
    let mut materializations = Vec::with_capacity(iterations);
    let mut last_visible = 0;
    let mut last_overscan = 0;
    let mut last_live = 0;
    let mut last_work = None;
    let table_bound = table_live_entity_bound();
    for iteration in 0..(warmup + iterations) {
        if timeout.is_some_and(|limit| loop_started.elapsed() > limit) {
            return skipped_scale(
                "table",
                logical_rows,
                Some(logical_columns),
                "materialize loop exceeded LARGE_SCALE_TIMEOUT",
            );
        }
        let scroll = (
            (iteration as f32 * 37.0) % 400.0,
            120.0 + (iteration as f32 * 131.0) % 4_000.0,
        );
        let window_started = Instant::now();
        let window = layout.window(scroll, TABLE_VIEWPORT, TABLE_OVERSCAN);
        let window_elapsed = window_started.elapsed();
        assert!(!window.rows.range.is_empty());
        assert!(!window.columns.range.is_empty());
        let materialize_started = Instant::now();
        let materialized = context
            .materialize_virtual_table(
                table,
                &mut items,
                &layout,
                scroll,
                TABLE_VIEWPORT,
                TABLE_OVERSCAN,
                |index| index,
                |index| index,
                |_index, _| TableRow::new(),
                |row, _, column, _| text_table_cell(row, column),
            )
            .unwrap();
        let materialize_elapsed = materialize_started.elapsed();
        let mounted_rows = materialized.rows.range.len();
        let mounted_columns = materialized.columns.range.len();
        let visible = layout
            .window(scroll, TABLE_VIEWPORT, (0.0, 0.0))
            .rows
            .range
            .len();
        let overscan = mounted_rows.saturating_sub(visible);
        let live = mounted_rows
            + items
                .mounted_rows()
                .iter()
                .map(|row| {
                    context
                        .world()
                        .node(items.row_entity(row).unwrap().stable_id())
                        .unwrap()
                        .children
                        .len()
                })
                .sum::<usize>();
        assert!(
            mounted_rows <= table_row_cap(),
            "virtual table mounted rows {mounted_rows} exceed geometric row cap {}",
            table_row_cap()
        );
        assert!(
            mounted_columns <= table_column_cap(),
            "virtual table mounted columns {mounted_columns} exceed geometric column cap {}",
            table_column_cap()
        );
        assert!(
            live <= table_bound,
            "virtual table live entities {live} exceed geometric bound {table_bound}"
        );
        let table_work = context.take_system_work();
        measure_virtual_table_text(&mut context, DocumentId::new(1).unwrap(), &table_work);
        if iteration >= warmup {
            windows.push(window_elapsed);
            materializations.push(materialize_elapsed);
            last_visible = visible;
            last_overscan = overscan;
            last_live = live;
            last_work = Some(ScaleWork::from(context.last_work_counters()));
        }
    }
    VirtualScaleCase {
        kind: "table",
        logical_rows,
        logical_columns: Some(logical_columns),
        status: "ok",
        skip_reason: None,
        visible_rows: Some(last_visible),
        overscan_rows: Some(last_overscan),
        cache_rows: 0,
        live_ui_entities: Some(last_live),
        live_ui_entities_bound: Some(table_bound),
        construction_ms: Some((construction_ms * 1_000.0).round() / 1_000.0),
        window_ms: Some(summarize(&windows)),
        materialize_ms: Some(summarize(&materializations)),
        work: last_work,
    }
}

fn drain_text(
    context: &mut AppContext,
    document: DocumentId,
    work: &SystemWork,
    viewport: LayoutViewport,
) {
    let mut shaper = MeasureTextShaper;
    let _ = context.resolve_styles(&work.style);
    let _ = context.shape_text(&work.text, &mut shaper);
    let _ = context.layout_document(document, viewport);
    let extracted = context.world().extract_nodes(&work.render_extraction);
    context.record_extract(&extracted);
}

fn bench_ime() -> CatalogWorkloadCase {
    let document = DocumentId::new(10).unwrap();
    let mut context = AppContext::new();
    let input = context
        .create_component(document, TextInput::new(""))
        .unwrap();
    assert!(context.focus_node(document, input.stable_id()).unwrap());
    let _ = context.take_system_work();
    let mut preedits = Vec::with_capacity(WORKLOAD_ITERATIONS);
    let mut commits = Vec::with_capacity(WORKLOAD_ITERATIONS);
    let mut last_work = None;
    for iteration in 0..(WORKLOAD_WARMUP + WORKLOAD_ITERATIONS) {
        let (script, preedit, commit) = IME_SCRIPTS[iteration % IME_SCRIPTS.len()];
        let started = Instant::now();
        assert!(
            context
                .set_ime_preedit(document, preedit.to_string(), None)
                .unwrap(),
            "ime preedit {script}"
        );
        let preedit_elapsed = started.elapsed();
        let _ = context.take_system_work();
        let started = Instant::now();
        assert!(
            context.commit_ime(document, commit).unwrap(),
            "ime commit {script}"
        );
        let commit_elapsed = started.elapsed();
        let work = context.take_system_work();
        drain_text(
            &mut context,
            document,
            &work,
            LayoutViewport::new(320.0, 40.0),
        );
        if iteration >= WORKLOAD_WARMUP {
            preedits.push(preedit_elapsed);
            commits.push(commit_elapsed);
            last_work = Some(ScaleWork::from(context.last_work_counters()));
        }
    }
    CatalogWorkloadCase {
        id: "ime",
        kind: "Ime",
        status: "ok",
        skip_reason: None,
        scripts: Some(IME_SCRIPTS.iter().map(|(script, _, _)| *script).collect()),
        panes: None,
        overlay_kinds: None,
        document_chars: None,
        visible_lines: None,
        live_ui_entities: Some(1),
        ime_script_count: Some(IME_SCRIPTS.len()),
        overlay_kind_count: None,
        preedit_ms: Some(summarize(&preedits)),
        commit_ms: Some(summarize(&commits)),
        resize_ms: None,
        activate_ms: None,
        local_edit_ms: None,
        work: last_work,
    }
}

fn eight_pane_root(contents: &[StableNodeId; DOCK_PANES]) -> DockNode {
    fn item(index: usize, content: StableNodeId) -> DockNode {
        DockNode::item(format!("pane-{index}"), Some(content))
    }
    fn split(axis: DockAxis, first: DockNode, second: DockNode) -> DockNode {
        DockNode::split(axis, 0.5, first, second)
    }
    split(
        DockAxis::Horizontal,
        split(
            DockAxis::Vertical,
            split(
                DockAxis::Horizontal,
                item(0, contents[0]),
                item(1, contents[1]),
            ),
            split(
                DockAxis::Horizontal,
                item(2, contents[2]),
                item(3, contents[3]),
            ),
        ),
        split(
            DockAxis::Vertical,
            split(
                DockAxis::Horizontal,
                item(4, contents[4]),
                item(5, contents[5]),
            ),
            split(
                DockAxis::Horizontal,
                item(6, contents[6]),
                item(7, contents[7]),
            ),
        ),
    )
}

fn bench_dock_workspace() -> CatalogWorkloadCase {
    let document = DocumentId::new(11).unwrap();
    let mut context = AppContext::new();
    let mut contents = [StableNodeId::new(1).unwrap(); DOCK_PANES];
    for (index, slot) in contents.iter_mut().enumerate() {
        *slot = context
            .create_component(document, Text::new(format!("pane {index}")))
            .unwrap()
            .stable_id();
    }
    let dock = context
        .create_component(document, Dock::new(eight_pane_root(&contents)))
        .unwrap();
    context.assemble_dock(dock).unwrap();
    let panes = context.read(dock, |dock| dock.flatten().len()).unwrap();
    assert_eq!(panes, DOCK_PANES);
    let work = context.take_system_work();
    drain_text(
        &mut context,
        document,
        &work,
        LayoutViewport::new(DOCK_VIEWPORT.0, DOCK_VIEWPORT.1),
    );
    let handle = context
        .world()
        .document_order(document)
        .into_iter()
        .find(|&id| context.is_dock_handle(id))
        .expect("assembled dock must expose a split handle");
    assert!(context.focus_node(document, handle).unwrap());
    let _ = context.take_system_work();
    let mut resizes = Vec::with_capacity(WORKLOAD_ITERATIONS);
    let mut last_work = None;
    for iteration in 0..(WORKLOAD_WARMUP + WORKLOAD_ITERATIONS) {
        let direction = if iteration.is_multiple_of(2) {
            1.0
        } else {
            -1.0
        };
        let started = Instant::now();
        assert!(context.focus_node(document, handle).unwrap());
        assert!(
            context
                .adjust_focused_dock_split(document, direction)
                .unwrap(),
            "dock splitter resize"
        );
        let resize_elapsed = started.elapsed();
        let work = context.take_system_work();
        drain_text(
            &mut context,
            document,
            &work,
            LayoutViewport::new(DOCK_VIEWPORT.0, DOCK_VIEWPORT.1),
        );
        if iteration >= WORKLOAD_WARMUP {
            resizes.push(resize_elapsed);
            last_work = Some(ScaleWork::from(context.last_work_counters()));
        }
    }
    let live = context.world().document_order(document).len();
    CatalogWorkloadCase {
        id: "dock-workspace",
        kind: "DockWorkspace",
        status: "ok",
        skip_reason: None,
        scripts: None,
        panes: Some(panes),
        overlay_kinds: None,
        document_chars: None,
        visible_lines: None,
        live_ui_entities: Some(live),
        ime_script_count: None,
        overlay_kind_count: None,
        preedit_ms: None,
        commit_ms: None,
        resize_ms: Some(summarize(&resizes)),
        activate_ms: None,
        local_edit_ms: None,
        work: last_work,
    }
}

fn bench_overlay() -> CatalogWorkloadCase {
    let document = DocumentId::new(12).unwrap();
    let mut context = AppContext::new();
    let host = context
        .create_component(document, OverlayHost::new())
        .unwrap();
    let tooltip = context
        .create_component(document, Tooltip::new("Hint"))
        .unwrap();
    let menu = context
        .create_component(document, Menu::new().label("Actions"))
        .unwrap();
    let dialog = context
        .create_component(document, Dialog::new("Settings"))
        .unwrap();
    let popover = context
        .create_component(document, Popover::new().trigger("Details"))
        .unwrap();
    context.append_child(host, tooltip).unwrap();
    context.append_child(host, menu).unwrap();
    context.append_child(host, dialog).unwrap();
    let _ = context.take_system_work();
    let mut activates = Vec::with_capacity(WORKLOAD_ITERATIONS);
    let mut last_work = None;
    for iteration in 0..(WORKLOAD_WARMUP + WORKLOAD_ITERATIONS) {
        let kind = OVERLAY_KINDS[iteration % OVERLAY_KINDS.len()];
        let started = Instant::now();
        match kind {
            "tooltip" => assert!(context.activate_overlay(host, tooltip).unwrap()),
            "context_menu" => assert!(context.activate_overlay(host, menu).unwrap()),
            "modal" => assert!(context.activate_overlay(host, dialog).unwrap()),
            "popup" => assert!(context.toggle_popover(popover).unwrap()),
            _ => unreachable!(),
        }
        let activate_elapsed = started.elapsed();
        let work = context.take_system_work();
        drain_text(
            &mut context,
            document,
            &work,
            LayoutViewport::new(640.0, 480.0),
        );
        if iteration >= WORKLOAD_WARMUP {
            activates.push(activate_elapsed);
            last_work = Some(ScaleWork::from(context.last_work_counters()));
        }
        match kind {
            "popup" => assert!(context.toggle_popover(popover).unwrap()),
            _ => {
                let _ = context.dismiss_overlay(host).unwrap();
            }
        }
        let _ = context.take_system_work();
    }
    CatalogWorkloadCase {
        id: "overlay",
        kind: "Overlay",
        status: "ok",
        skip_reason: None,
        scripts: None,
        panes: None,
        overlay_kinds: Some(OVERLAY_KINDS.to_vec()),
        document_chars: None,
        visible_lines: None,
        live_ui_entities: Some(context.world().document_order(document).len()),
        ime_script_count: None,
        overlay_kind_count: Some(OVERLAY_KINDS.len()),
        preedit_ms: None,
        commit_ms: None,
        resize_ms: None,
        activate_ms: Some(summarize(&activates)),
        local_edit_ms: None,
        work: last_work,
    }
}

fn editor_document(chars: usize) -> String {
    let mut text = String::with_capacity(chars);
    let mut row = 0usize;
    while text.len() < chars {
        text.push_str(&format!("row {row:04} local edit line\n"));
        row += 1;
    }
    text.truncate(chars);
    text
}

fn bench_text_editor() -> CatalogWorkloadCase {
    let document = DocumentId::new(13).unwrap();
    let mut context = AppContext::new();
    let value = editor_document(TEXT_EDITOR_CHARS);
    assert_eq!(value.len(), TEXT_EDITOR_CHARS);
    let area = context
        .create_component(
            document,
            TextArea::new(value).height(TEXT_EDITOR_VISIBLE_LINES as f32 * TEXT_EDITOR_LINE_PX),
        )
        .unwrap();
    assert!(context.focus_node(document, area.stable_id()).unwrap());
    let work = context.take_system_work();
    drain_text(
        &mut context,
        document,
        &work,
        LayoutViewport::new(
            800.0,
            TEXT_EDITOR_VISIBLE_LINES as f32 * TEXT_EDITOR_LINE_PX,
        ),
    );
    let mut edits = Vec::with_capacity(WORKLOAD_ITERATIONS);
    let mut last_work = None;
    for iteration in 0..(WORKLOAD_WARMUP + WORKLOAD_ITERATIONS) {
        let patch = if iteration.is_multiple_of(2) {
            "x"
        } else {
            "y"
        };
        let started = Instant::now();
        assert!(context.replace_text_area_selection(area, patch).unwrap());
        let edit_elapsed = started.elapsed();
        let work = context.take_system_work();
        drain_text(
            &mut context,
            document,
            &work,
            LayoutViewport::new(
                800.0,
                TEXT_EDITOR_VISIBLE_LINES as f32 * TEXT_EDITOR_LINE_PX,
            ),
        );
        if iteration >= WORKLOAD_WARMUP {
            edits.push(edit_elapsed);
            last_work = Some(ScaleWork::from(context.last_work_counters()));
        }
    }
    CatalogWorkloadCase {
        id: "text-editor",
        kind: "TextEditor",
        status: "ok",
        skip_reason: None,
        scripts: None,
        panes: None,
        overlay_kinds: None,
        document_chars: Some(TEXT_EDITOR_CHARS),
        visible_lines: Some(TEXT_EDITOR_VISIBLE_LINES),
        live_ui_entities: Some(1),
        ime_script_count: None,
        overlay_kind_count: None,
        preedit_ms: None,
        commit_ms: None,
        resize_ms: None,
        activate_ms: None,
        local_edit_ms: Some(summarize(&edits)),
        work: last_work,
    }
}
