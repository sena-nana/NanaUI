//! Scene-only scaling observation. This is not a Surface/120 Hz acceptance run.
use super::*;
use std::time::Duration;

#[derive(Serialize)]
struct Sample {
    retained_nodes: usize,
    visible_operations: usize,
    frames: usize,
    sampling_seconds: f64,
    initial_scene_ms: f64,
    local_update_cpu_ms: Timing,
    scroll_cpu_ms: Timing,
    visible_query_cpu_ms: Timing,
    rebuilt_descendant_primitives_on_scroll: usize,
    structural_plan_recompiles: usize,
}

#[derive(Serialize)]
struct Timing {
    p50: f64,
    p95: f64,
    p99: f64,
    max: f64,
}

fn timing(values: &[f64]) -> Timing {
    let value = summarize(values);
    Timing {
        p50: value.p50,
        p95: value.p95,
        p99: value.p99,
        max: value.max,
    }
}

pub(super) fn run(output: Option<String>) {
    let samples = [10_000, 50_000, 100_000].map(sample);
    let report = serde_json::json!({
        "schema_version": 1,
        "scope": "scene-only affine quads; no text, Runtime, GPU or Surface timing",
        "profile": if cfg!(debug_assertions) {"debug"} else {"release"},
        "sampling": "60 seconds per scale, serial, requested cadence 120 Hz; sleep is not presentation",
        "samples": samples,
    });
    let json = serde_json::to_string_pretty(&report).unwrap();
    if let Some(path) = output {
        let path = std::path::PathBuf::from(path);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(path, json).unwrap();
    } else {
        println!("{json}");
    }
}

fn sample(count: usize) -> Sample {
    let mut nodes = build_nodes(count);
    // All scales have exactly the same visible geometry.
    for node in nodes.iter_mut().skip(1) {
        Arc::make_mut(&mut node.source_style.layout).background = Some([0.3, 0.2, 0.4, 1.0]);
    }
    let mut root = nodes[0].clone();
    let mut scene = UiScene::new();
    let started = Instant::now();
    scene.apply_delta(nodes, []);
    let plan = scene.frame_plan().unwrap();
    let viewport = nana_ui_scene::SceneRect {
        x: 0.0,
        y: 0.0,
        width: 1920.0,
        height: 1080.0,
    };
    scene.visible_operations(viewport).unwrap();
    let initial = elapsed_ms(started);
    let mut local = Vec::with_capacity(7200);
    let mut scroll = Vec::with_capacity(7200);
    let mut visible = Vec::with_capacity(7200);
    let mut descendant_rebuilds = 0;
    let mut recompiles = 0;
    let mut frame = 0;
    let mut visible_count = 0;
    let warmup = Instant::now();
    let mut sampling = None;
    loop {
        let frame_start = Instant::now();
        if sampling.is_none() && warmup.elapsed() >= Duration::from_secs(2) {
            sampling = Some(frame_start);
        }
        if sampling.is_some_and(|start| start.elapsed() >= Duration::from_secs(60)) {
            break;
        }
        let mut changed = leaf(3, if frame % 2 == 0 { 0.4 } else { 0.5 });
        changed.parent = Some(id(1));
        let start = Instant::now();
        let delta = scene.apply_delta([changed], []);
        assert_eq!(delta.rebuilt_primitives, 1);
        let local_ms = elapsed_ms(start);
        root.scroll_offset.y = (frame % 2) as f32;
        let start = Instant::now();
        let delta = scene.apply_delta([root.clone()], []);
        let scroll_ms = elapsed_ms(start);
        let start = Instant::now();
        visible_count = black_box(scene.visible_operations(viewport).unwrap()).len();
        let visible_ms = elapsed_ms(start);
        if sampling.is_some() {
            local.push(local_ms);
            scroll.push(scroll_ms);
            visible.push(visible_ms);
            descendant_rebuilds += delta.rebuilt_primitives;
            recompiles += usize::from(!Arc::ptr_eq(&plan, &scene.frame_plan().unwrap()));
        }
        frame += 1;
        std::thread::sleep(
            Duration::from_secs_f64(1.0 / 120.0).saturating_sub(frame_start.elapsed()),
        );
    }
    assert_eq!(descendant_rebuilds, 0);
    assert_eq!(recompiles, 0);
    Sample {
        retained_nodes: count + 1,
        visible_operations: visible_count,
        frames: local.len(),
        sampling_seconds: sampling.unwrap().elapsed().as_secs_f64(),
        initial_scene_ms: initial,
        local_update_cpu_ms: timing(&local),
        scroll_cpu_ms: timing(&scroll),
        visible_query_cpu_ms: timing(&visible),
        rebuilt_descendant_primitives_on_scroll: descendant_rebuilds,
        structural_plan_recompiles: recompiles,
    }
}
