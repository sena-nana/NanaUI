use std::fs;
use std::hint::black_box;
use std::sync::Arc;
use std::time::Instant;

use nana_ui_core::LayoutStyle;
use nana_ui_runtime::{ComputedStyle, ExtractedNode, LayoutBox, NodeKind, NodeStyle, StableNodeId};
use nana_ui_scene::{ResourceId, UiScene};
use serde::Serialize;

#[derive(Serialize)]
struct Report {
    phase: &'static str,
    profile: &'static str,
    samples: usize,
    rows: Vec<Row>,
}

#[derive(Serialize)]
struct Row {
    nodes: usize,
    primitives: usize,
    initial_extraction_ms: f64,
    local_update_p95_ms: f64,
    idle_update_p95_ms: f64,
    frame_graph_p95_ms: f64,
}

fn main() {
    let output = std::env::args().skip_while(|arg| arg != "--output").nth(1);
    let report = Report {
        phase: "issue-7-phase-6",
        profile: "release",
        samples: 200,
        rows: [100, 500, 1000, 5000].into_iter().map(benchmark).collect(),
    };
    let json = serde_json::to_string_pretty(&report).expect("serialize report") + "\n";
    if let Some(path) = output {
        fs::write(path, json).expect("write report");
    } else {
        print!("{json}");
    }
}

fn benchmark(nodes: usize) -> Row {
    let initial = build_nodes(nodes);
    let mut scene = UiScene::new();
    let started = Instant::now();
    let delta = scene.apply_delta(initial, []);
    let initial_extraction_ms = elapsed_ms(started);
    assert_eq!(delta.updated_nodes, nodes + 1);

    let target = nodes / 2 + 2;
    let mut local = Vec::with_capacity(200);
    for revision in 0..200 {
        let mut node = leaf(target as u64, revision as f32 / 200.0);
        node.parent = Some(id(1));
        let started = Instant::now();
        let delta = scene.apply_delta([node], []);
        local.push(elapsed_ms(started));
        assert!(!delta.order_rebuilt);
        assert_eq!(delta.rebuilt_primitives, 1);
    }

    let mut idle = Vec::with_capacity(200);
    for _ in 0..200 {
        let started = Instant::now();
        let delta = scene.apply_delta([], []);
        idle.push(elapsed_ms(started));
        assert_eq!(delta.updated_nodes, 0);
        assert_eq!(delta.rebuilt_primitives, 0);
    }

    let mut graph = Vec::with_capacity(60);
    for _ in 0..60 {
        let started = Instant::now();
        let compiled = scene.frame_graph(ResourceId(1)).unwrap();
        black_box(compiled);
        graph.push(elapsed_ms(started));
    }
    Row {
        nodes,
        primitives: scene.primitives().count(),
        initial_extraction_ms,
        local_update_p95_ms: percentile(&mut local, 0.95),
        idle_update_p95_ms: percentile(&mut idle, 0.95),
        frame_graph_p95_ms: percentile(&mut graph, 0.95),
    }
}

fn build_nodes(nodes: usize) -> Vec<ExtractedNode> {
    let children = (2..=nodes as u64 + 1).map(id).collect::<Vec<_>>();
    let mut root = leaf(1, 0.0);
    root.source_style = NodeStyle::default();
    root.children = children;
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
        kind: NodeKind::Element { tag: "div".into() },
        parent: None,
        children: Vec::new(),
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
            ..Default::default()
        },
        style: ComputedStyle::default(),
        text: None,
        text_metrics: None,
        z_index: 0,
        focused: false,
        ime: None,
        text_input: None,
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

fn percentile(samples: &mut [f64], percentile: f64) -> f64 {
    samples.sort_by(f64::total_cmp);
    samples[((samples.len() - 1) as f64 * percentile).ceil() as usize]
}
