use std::fs;
use std::hint::black_box;
use std::sync::Arc;
use std::time::Instant;

use nana_ui_core::LayoutStyle;
use nana_ui_runtime::{ComputedStyle, ExtractedNode, LayoutBox, NodeKind, NodeStyle, StableNodeId};
use nana_ui_scene::{ResourceId, UiScene};
use serde::Serialize;

const FRAME_BUDGET_MS: f64 = 16.67;

#[derive(Serialize)]
struct Report {
    schema_version: u32,
    phase: &'static str,
    profile: &'static str,
    frame_budget_ms: f64,
    frame_budget_hz: u32,
    samples: usize,
    rows: Vec<Row>,
}

#[derive(Serialize)]
struct Row {
    nodes: usize,
    kind: &'static str,
    primitives: usize,
    initial_extraction_ms: Distribution,
    #[serde(skip_serializing_if = "Option::is_none")]
    local_update_ms: Option<Distribution>,
    #[serde(skip_serializing_if = "Option::is_none")]
    idle_update_ms: Option<Distribution>,
    #[serde(skip_serializing_if = "Option::is_none")]
    frame_graph_ms: Option<Distribution>,
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
    let output = std::env::args().skip_while(|arg| arg != "--output").nth(1);
    let mut rows = [100, 500, 1000, 5000]
        .into_iter()
        .map(|nodes| benchmark_full(nodes, 200, 60))
        .collect::<Vec<_>>();
    rows.push(benchmark_full(10_000, 50, 20));
    rows.push(benchmark_construction(50_000, 8));
    let report = Report {
        schema_version: 2,
        phase: "issue-8-scale",
        profile: "release",
        frame_budget_ms: FRAME_BUDGET_MS,
        frame_budget_hz: 60,
        samples: 200,
        rows,
    };
    let json = serde_json::to_string_pretty(&report).expect("serialize report") + "\n";
    if let Some(path) = output {
        let path = std::path::PathBuf::from(path);
        if let Some(parent) = path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
        {
            fs::create_dir_all(parent).expect("write report directory");
        }
        fs::write(path, json).expect("write report");
    } else {
        print!("{json}");
    }
}

fn benchmark_full(nodes: usize, mutation_samples: usize, graph_samples: usize) -> Row {
    let initial = build_nodes(nodes);
    let mut scene = UiScene::new();
    let started = Instant::now();
    let delta = scene.apply_delta(initial, []);
    let initial_extraction_ms = summarize(&[elapsed_ms(started)]);
    assert_eq!(delta.updated_nodes, nodes + 1);

    let target = nodes / 2 + 2;
    let mut local = Vec::with_capacity(mutation_samples);
    for revision in 0..mutation_samples {
        let mut node = leaf(target as u64, revision as f32 / mutation_samples as f32);
        node.parent = Some(id(1));
        let started = Instant::now();
        let delta = scene.apply_delta([node], []);
        local.push(elapsed_ms(started));
        assert!(!delta.order_rebuilt);
        assert_eq!(delta.rebuilt_primitives, 1);
    }

    let mut idle = Vec::with_capacity(mutation_samples);
    for _ in 0..mutation_samples {
        let started = Instant::now();
        let delta = scene.apply_delta([], []);
        idle.push(elapsed_ms(started));
        assert_eq!(delta.updated_nodes, 0);
        assert_eq!(delta.rebuilt_primitives, 0);
    }

    let mut graph = Vec::with_capacity(graph_samples);
    for _ in 0..graph_samples {
        let started = Instant::now();
        let compiled = scene.frame_graph(ResourceId(1)).unwrap();
        black_box(compiled);
        graph.push(elapsed_ms(started));
    }
    Row {
        nodes,
        kind: "full",
        primitives: scene.primitives().count(),
        initial_extraction_ms,
        local_update_ms: Some(summarize(&local)),
        idle_update_ms: Some(summarize(&idle)),
        frame_graph_ms: Some(summarize(&graph)),
    }
}

fn benchmark_construction(nodes: usize, samples: usize) -> Row {
    let mut extraction = Vec::with_capacity(samples);
    let mut primitives = 0;
    for _ in 0..samples {
        let initial = build_nodes(nodes);
        let mut scene = UiScene::new();
        let started = Instant::now();
        let delta = scene.apply_delta(initial, []);
        extraction.push(elapsed_ms(started));
        assert_eq!(delta.updated_nodes, nodes + 1);
        primitives = scene.primitives().count();
    }
    Row {
        nodes,
        kind: "construction",
        primitives,
        initial_extraction_ms: summarize(&extraction),
        local_update_ms: None,
        idle_update_ms: None,
        frame_graph_ms: None,
    }
}

fn build_nodes(nodes: usize) -> Vec<ExtractedNode> {
    let children = (2..=nodes as u64 + 1).map(id).collect::<Vec<_>>();
    let mut root = leaf(1, 0.0);
    root.source_style = NodeStyle::default();
    root.children = Arc::new(children);
    let mut extracted = Vec::with_capacity(nodes + 1);
    extracted.push(root);
    for value in 2..=nodes as u64 + 1 {
        let mut node = leaf(value, value as f32 / nodes as f32);
        node.parent = Some(id(1));
        extracted.push(node);
    }
    extracted
}

fn leaf(value: u64, shade: f32) -> ExtractedNode {
    ExtractedNode {
        id: id(value),
        kind: Arc::new(NodeKind::Element { tag: "div".into() }),
        parent: None,
        children: Arc::new(Vec::new()),
        layout: LayoutBox {
            x: 0.0,
            y: value as f32 * 2.0,
            width: 160.0,
            height: 2.0,
        },
        scroll_offset: nana_ui_runtime::ScrollOffset::default(),
        source_style: NodeStyle {
            layout: Arc::new(LayoutStyle {
                background: Some([shade, 0.2, 0.4, 1.0]),
                ..Default::default()
            }),
            ..NodeStyle::default()
        },
        style: Arc::new(ComputedStyle::default()),
        text: None,
        text_metrics: None,
        z_index: 0,
        focused: false,
        ime: None,
        text_input: None,
        text_spans: Vec::new(),
        standard_visual: None,
        component_geometry: None,
        standard_visual_foreground: None,
        custom_render: None,
    }
}

fn id(value: u64) -> StableNodeId {
    StableNodeId::new(value).unwrap()
}

fn elapsed_ms(started: Instant) -> f64 {
    started.elapsed().as_secs_f64() * 1000.0
}

fn summarize(samples: &[f64]) -> Distribution {
    let mut values = samples.to_vec();
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

fn percentile(samples: &[f64], percentile: f64) -> f64 {
    samples[((samples.len() - 1) as f64 * percentile).round() as usize]
}
