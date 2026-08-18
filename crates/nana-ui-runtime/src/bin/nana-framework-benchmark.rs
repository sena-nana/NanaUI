use std::time::{Duration, Instant};

use nana_ui_core::{TableColumn, VirtualListLayout, VirtualTableLayout};
use nana_ui_runtime::{
    Activate, AppContext, Button, ContextPredicate, DocumentId, KeyContext, LayoutViewport, List,
    NodeKind, ScrollAxes, ScrollOffset, ScrollView, Table, TableCell, TableRow, Text, TextContent,
    VirtualListItems, VirtualTableItems,
};
use serde::Serialize;

const WARMUP_ITERATIONS: usize = 100;
const ITERATIONS: usize = 1_000;
const FRAME_BUDGET_MS: f64 = 16.67;
const SCALE_100K_WARMUP: usize = 10;
const SCALE_100K_ITERATIONS: usize = 40;
const SCALE_1M_WARMUP: usize = 5;
const SCALE_1M_ITERATIONS: usize = 15;
const LARGE_SCALE_TIMEOUT: Duration = Duration::from_secs(90);
const LIST_VIEWPORT: f32 = 800.0;
const LIST_OVERSCAN: f32 = 200.0;
const LIST_ITEM_EXTENT: f32 = 20.0;
const TABLE_VIEWPORT: (f32, f32) = (1_280.0, 800.0);
const TABLE_OVERSCAN: (f32, f32) = (160.0, 200.0);
const TABLE_COLUMN_EXTENT: f32 = 80.0;

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
                |row, _, column, _| TableCell::new(format!("{row}:{column}")),
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
                |row, _, column, _| TableCell::new(format!("{row}:{column}")),
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
    }
}
