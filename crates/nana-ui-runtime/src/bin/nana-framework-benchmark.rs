use std::time::{Duration, Instant};

use nana_ui_core::{TableColumn, VirtualListLayout, VirtualTableLayout};
use nana_ui_runtime::{
    Activate, AppContext, Button, ContextPredicate, DocumentId, KeyContext, List, NodeKind,
    ScrollAxes, ScrollOffset, ScrollView, Table, TableCell, TableRow, Text, TextContent,
    VirtualListItems, VirtualTableItems,
};
use serde::Serialize;

const WARMUP_ITERATIONS: usize = 100;
const ITERATIONS: usize = 1_000;

#[derive(Default)]
struct Counter(usize);

struct Increment;

#[derive(Serialize)]
struct Report {
    schema_version: u32,
    profile: &'static str,
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
}

#[derive(Serialize)]
struct Distribution {
    p50: f64,
    p95: f64,
    p99: f64,
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
    let mut virtual_list = VirtualListLayout::new(std::iter::repeat_n(20.0, 10_000));
    let mut virtual_table = VirtualTableLayout::new(
        std::iter::repeat_n(20.0, 10_000),
        (0..100).map(|index| TableColumn::new(format!("column-{index}"), 80.0)),
    );
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
        let window = virtual_list.window((iteration * 13) as f32, 800.0, 200.0);
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
                800.0,
                200.0,
                |index| index,
                |index, _| Text::new(format!("Visible row {index}")),
            )
            .unwrap();
        let virtual_list_materialize_elapsed = started.elapsed();
        assert!(materialized.range.len() < 100);
        assert_eq!(
            context
                .world()
                .node(materialized_list.stable_id())
                .unwrap()
                .children
                .len(),
            materialized.range.len()
        );
        let _ = context.take_system_work();
        let started = Instant::now();
        let table_window = virtual_table.window(
            ((iteration * 37) as f32, (iteration * 131) as f32),
            (1_280.0, 800.0),
            (160.0, 200.0),
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
                (1_280.0, 800.0),
                (160.0, 200.0),
                |index| index,
                |index| index,
                |_index, _| TableRow::new(),
                |row, _, column, _| TableCell::new(format!("{row}:{column}")),
            )
            .unwrap();
        let virtual_table_materialize_elapsed = started.elapsed();
        let visible_rows = materialized_table_window.rows.range.len();
        let visible_columns = materialized_table_window.columns.range.len();
        assert!(visible_rows < 100);
        assert!(visible_columns < 30);
        assert_eq!(
            context
                .world()
                .node(materialized_table.stable_id())
                .unwrap()
                .children
                .len(),
            visible_rows
        );
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
            visible_rows * visible_columns
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
        }
    }
    write_report(&Report {
        schema_version: 7,
        profile: if cfg!(debug_assertions) {
            "debug"
        } else {
            "release"
        },
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
    }
}

fn percentile(values: &[f64], percentile: f64) -> f64 {
    let index = ((values.len() - 1) as f64 * percentile).round() as usize;
    (values[index] * 1_000.0).round() / 1_000.0
}
