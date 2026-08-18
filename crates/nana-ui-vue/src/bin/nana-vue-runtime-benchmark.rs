use std::time::{Duration, Instant};

use nana_ui_core::{AppearanceSettings, ThemeMode};
use nana_ui_vue::{
    DocumentId, LayoutBox, NanaTreeDocument, NodeHandle, SemanticSnapshot, SemanticWidget,
    WidgetKind, WidgetProps,
};
use serde::Serialize;

const FRAME_BUDGET_MS: f64 = 16.67;
const SMALL_WARMUP: usize = 10;
const SMALL_ITERATIONS: usize = 60;
const SCALE_10K_WARMUP: usize = 5;
const SCALE_10K_ITERATIONS: usize = 20;
const CONSTRUCTION_WARMUP: usize = 2;
const CONSTRUCTION_ITERATIONS: usize = 8;

#[derive(Serialize)]
struct Report {
    schema_version: u32,
    profile: &'static str,
    frame_budget_ms: f64,
    frame_budget_hz: u32,
    warmup_iterations: usize,
    iterations: usize,
    cases: Vec<Case>,
}

#[derive(Serialize)]
struct Case {
    nodes: usize,
    kind: &'static str,
    warmup_iterations: usize,
    iterations: usize,
    construction_ms: Distribution,
    #[serde(skip_serializing_if = "Option::is_none")]
    initial_semantic_ms: Option<Distribution>,
    #[serde(skip_serializing_if = "Option::is_none")]
    idle_semantic_ms: Option<Distribution>,
    #[serde(skip_serializing_if = "Option::is_none")]
    initial_layout_ms: Option<Distribution>,
    #[serde(skip_serializing_if = "Option::is_none")]
    idle_layout_ms: Option<Distribution>,
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
    let mut cases = Vec::new();
    for nodes in [100, 500, 1_000, 5_000] {
        cases.push(bench_full(nodes, SMALL_WARMUP, SMALL_ITERATIONS));
    }
    cases.push(bench_full(10_000, SCALE_10K_WARMUP, SCALE_10K_ITERATIONS));
    cases.push(bench_construction(
        50_000,
        CONSTRUCTION_WARMUP,
        CONSTRUCTION_ITERATIONS,
    ));
    write_report(&Report {
        schema_version: 3,
        profile: if cfg!(debug_assertions) {
            "debug"
        } else {
            "release"
        },
        frame_budget_ms: FRAME_BUDGET_MS,
        frame_budget_hz: 60,
        warmup_iterations: SMALL_WARMUP,
        iterations: SMALL_ITERATIONS,
        cases,
    });
}

fn bench_full(nodes: usize, warmup: usize, iterations: usize) -> Case {
    let mut construction = Vec::with_capacity(iterations);
    let mut initial_semantic = Vec::with_capacity(iterations);
    let mut idle_semantic = Vec::with_capacity(iterations);
    let mut initial_layout = Vec::with_capacity(iterations);
    let mut idle_layout = Vec::with_capacity(iterations);
    for iteration in 0..(warmup + iterations) {
        let started = Instant::now();
        let (mut document, snapshot, boxes) = build_tree(nodes);
        let construction_elapsed = started.elapsed();

        let started = Instant::now();
        document.sync_semantic_styles(&snapshot);
        let initial_semantic_elapsed = started.elapsed();
        let generation = document.runtime_generation();
        let started = Instant::now();
        document.sync_semantic_styles(&snapshot);
        let idle_semantic_elapsed = started.elapsed();
        assert_eq!(document.runtime_generation(), generation);

        let started = Instant::now();
        document.apply_layout_boxes(&boxes);
        let initial_layout_elapsed = started.elapsed();
        let generation = document.runtime_generation();
        let started = Instant::now();
        document.apply_layout_boxes(&boxes);
        let idle_layout_elapsed = started.elapsed();
        assert_eq!(document.runtime_generation(), generation);

        if iteration >= warmup {
            construction.push(construction_elapsed);
            initial_semantic.push(initial_semantic_elapsed);
            idle_semantic.push(idle_semantic_elapsed);
            initial_layout.push(initial_layout_elapsed);
            idle_layout.push(idle_layout_elapsed);
        }
    }
    Case {
        nodes,
        kind: "full",
        warmup_iterations: warmup,
        iterations,
        construction_ms: summarize(&construction),
        initial_semantic_ms: Some(summarize(&initial_semantic)),
        idle_semantic_ms: Some(summarize(&idle_semantic)),
        initial_layout_ms: Some(summarize(&initial_layout)),
        idle_layout_ms: Some(summarize(&idle_layout)),
    }
}

fn bench_construction(nodes: usize, warmup: usize, iterations: usize) -> Case {
    let mut construction = Vec::with_capacity(iterations);
    for iteration in 0..(warmup + iterations) {
        let started = Instant::now();
        let _ = build_tree(nodes);
        let construction_elapsed = started.elapsed();
        if iteration >= warmup {
            construction.push(construction_elapsed);
        }
    }
    Case {
        nodes,
        kind: "construction",
        warmup_iterations: warmup,
        iterations,
        construction_ms: summarize(&construction),
        initial_semantic_ms: None,
        idle_semantic_ms: None,
        initial_layout_ms: None,
        idle_layout_ms: None,
    }
}

fn build_tree(
    nodes: usize,
) -> (
    NanaTreeDocument,
    SemanticSnapshot,
    Vec<(NodeHandle, LayoutBox)>,
) {
    let mut document = NanaTreeDocument::with_id(DocumentId(1), 1280, 720, 1.0);
    let mut widgets = Vec::with_capacity(nodes);
    let mut boxes = Vec::with_capacity(nodes);
    for index in 0..nodes {
        let handle = document.create_element("button");
        document.insert(handle, document.mount_root(), None);
        widgets.push(SemanticWidget {
            id: handle.0,
            kind: WidgetKind::Button,
            props: WidgetProps::default(),
            children: Vec::new(),
            parent: Some(document.mount_root().0),
        });
        boxes.push((
            handle,
            LayoutBox {
                handle,
                x: (index % 50) as f32 * 20.0,
                y: (index / 50) as f32 * 20.0,
                width: 18.0,
                height: 18.0,
            },
        ));
    }
    let snapshot = SemanticSnapshot {
        revision: 1,
        theme: ThemeMode::Light,
        appearance: AppearanceSettings::default(),
        roots: vec![document.mount_root().0],
        widgets,
    };
    (document, snapshot, boxes)
}

fn elapsed_ms(duration: Duration) -> f64 {
    (duration.as_secs_f64() * 1_000_000.0).round() / 1_000.0
}

fn summarize(samples: &[Duration]) -> Distribution {
    let mut values = samples.iter().copied().map(elapsed_ms).collect::<Vec<_>>();
    values.sort_by(f64::total_cmp);
    Distribution {
        p50: percentile(&values, 0.50),
        p95: percentile(&values, 0.95),
        p99: percentile(&values, 0.99),
        max: values.last().copied().unwrap_or(0.0),
        frame_budget_ms: FRAME_BUDGET_MS,
        frame_budget_misses: values
            .iter()
            .filter(|value| **value > FRAME_BUDGET_MS)
            .count(),
    }
}

fn percentile(values: &[f64], percentile: f64) -> f64 {
    values[((values.len() - 1) as f64 * percentile).round() as usize]
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
