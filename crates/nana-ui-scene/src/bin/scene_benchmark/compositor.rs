//! Issue #87 compositor work-counter catalog. Timing is supplementary; Weekly
//! GHA is not a fixed machine. Do not invent GPU encode timings here.
use super::*;
use std::time::Duration;

use nana_ui_core::{PaintTransform, motion::MotionWorkCounters};
use nana_ui_runtime::{
    AnimatableProperty, AnimationId, AnimationSpec, DocumentId, Easing, MotionInterrupt, MotionTo,
    MotionValue, MutationQueue, UiWorld,
};
use nana_ui_scene::LAYER_PROMOTE_HOLD;
use serde::Serialize;

const STEADY_MS: u64 = 80;
const DURATION_MS: u64 = 400;

#[derive(Serialize)]
struct CompositorReport {
    schema_version: u32,
    phase: &'static str,
    profile: &'static str,
    catalog_compositor: CatalogCompositor,
    notes: Vec<&'static str>,
}

#[derive(Serialize)]
struct CatalogCompositor {
    steady: CompositorCase,
    scales: Vec<CompositorCase>,
    retarget: CompositorCase,
    churn: CompositorCase,
}

#[derive(Serialize)]
struct CompositorCase {
    id: &'static str,
    kind: &'static str,
    status: &'static str,
    tracks: usize,
    properties: &'static str,
    workload: &'static str,
    work: MotionWorkSnapshot,
    #[serde(skip_serializing_if = "Option::is_none")]
    retargets: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    start_stop: Option<usize>,
    steady_ms: super::Distribution,
}

#[derive(Serialize, Clone, Copy)]
struct MotionWorkSnapshot {
    motion_tracks_active: usize,
    motion_tracks_cpu: usize,
    motion_tracks_compositor: usize,
    presentation_values_cpu_sampled: usize,
    compositor_layers_active: usize,
    compositor_layers_promoted: usize,
    compositor_layers_demoted: usize,
    compositor_cache_bytes: usize,
    uiworld_mutations_from_animation: usize,
    layout_nodes_from_animation: usize,
    style_processed_from_animation: usize,
    render_nodes_reextracted_from_animation: usize,
    animations_considered: usize,
    animation_deadlines_scanned: usize,
}

#[derive(Clone, Copy)]
enum PropertySet {
    Transform,
    Opacity,
    Mixed,
}

impl PropertySet {
    fn label(self) -> &'static str {
        match self {
            Self::Transform => "transform",
            Self::Opacity => "opacity",
            Self::Mixed => "mixed",
        }
    }
}

pub(super) fn run(output: Option<String>) {
    let document = DocumentId::new(1).unwrap();
    let steady = measure_scale(
        document,
        1,
        PropertySet::Opacity,
        "compositor-steady",
        "steady",
        8,
        24,
    );
    let mut scales = Vec::new();
    for (tracks, warmup, iterations) in [
        (1usize, 6usize, 20usize),
        (100, 4, 12),
        (1_000, 2, 6),
        (10_000, 1, 3),
    ] {
        for properties in [
            PropertySet::Transform,
            PropertySet::Opacity,
            PropertySet::Mixed,
        ] {
            scales.push(measure_scale(
                document,
                tracks,
                properties,
                "compositor-tracks",
                "scale",
                warmup,
                iterations,
            ));
        }
    }
    let report = CompositorReport {
        schema_version: 1,
        phase: "issue-87-compositor",
        profile: if cfg!(debug_assertions) {
            "debug"
        } else {
            "release"
        },
        catalog_compositor: CatalogCompositor {
            steady,
            scales,
            retarget: measure_retarget(document, 100),
            churn: measure_churn(document, 100),
        },
        notes: vec![
            "Work counters are the Issue #87 §13 gate. steady_ms is a host observation, not a public CI GPU timing.",
            "motion_descriptors_uploaded is omitted: this binary does not encode/submit.",
            "compositor_cache_bytes is 0: no offscreen raster cache; primitives stay geometry.",
            "Weekly GHA is not a fixed benchmark machine.",
        ],
    };
    let json = serde_json::to_string_pretty(&report).expect("serialize compositor report") + "\n";
    if let Some(path) = output {
        let path = std::path::PathBuf::from(path);
        if let Some(parent) = path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
        {
            fs::create_dir_all(parent).expect("write report directory");
        }
        fs::write(&path, json).expect("write compositor report");
        println!("{}", path.display());
    } else {
        print!("{json}");
    }
}

fn measure_scale(
    document: DocumentId,
    tracks: usize,
    properties: PropertySet,
    id: &'static str,
    workload: &'static str,
    warmup: usize,
    iterations: usize,
) -> CompositorCase {
    let mut samples = Vec::with_capacity(iterations);
    let mut last = None;
    for iteration in 0..(warmup + iterations) {
        let mut world = UiWorld::new();
        let ids = install_tracks(&mut world, document, tracks, properties);
        let mut scene = promote(&mut world, &ids);
        let queries = world.presentation_values_cpu_sampled();
        let started = Instant::now();
        let frame = world.advance_animations(Duration::from_millis(STEADY_MS));
        let elapsed = started.elapsed();
        let work = merge_frame(&mut world, &mut scene, queries, &frame);
        assert!(
            quiet(work),
            "compositor {id} {tracks} {} must stay quiet: {work:?}",
            properties.label()
        );
        last = Some((
            work,
            frame.animations_considered,
            frame.animation_deadlines_scanned,
        ));
        if iteration >= warmup {
            samples.push(elapsed.as_secs_f64() * 1_000.0);
        }
    }
    let (work, considered, scanned) = last.expect("compositor scale must observe a frame");
    CompositorCase {
        id,
        kind: "Animation",
        status: "ok",
        tracks,
        properties: properties.label(),
        workload,
        work: snapshot(work, considered, scanned),
        retargets: None,
        start_stop: None,
        steady_ms: super::summarize(&samples),
    }
}

fn measure_retarget(document: DocumentId, tracks: usize) -> CompositorCase {
    let mut samples = Vec::new();
    let mut last = None;
    for iteration in 0..8 {
        let mut world = UiWorld::new();
        let ids = install_tracks(&mut world, document, tracks, PropertySet::Opacity);
        let mut scene = promote(&mut world, &ids);
        let mut storm = MutationQueue::new();
        for (index, node) in ids.iter().enumerate() {
            storm.start_animation(
                opacity_spec((index + 1) as u64, *node, 80, 0.0)
                    .with_interrupt(MotionInterrupt::Retarget),
            );
        }
        world.commit(storm).unwrap();
        let _ = world.take_system_work();
        // Retarget may enqueue a start deadline. The gated frame is the
        // compositor present after that mutation, not the due-start tick.
        drain_due(&mut world, &mut scene, Duration::from_millis(80));
        let queries = world.presentation_values_cpu_sampled();
        let started = Instant::now();
        let frame = world.advance_animations(Duration::from_millis(120));
        let elapsed = started.elapsed();
        let work = merge_frame(&mut world, &mut scene, queries, &frame);
        assert!(quiet(work), "retarget steady must stay quiet: {work:?}");
        last = Some((
            work,
            frame.animations_considered,
            frame.animation_deadlines_scanned,
        ));
        if iteration >= 2 {
            samples.push(elapsed.as_secs_f64() * 1_000.0);
        }
    }
    let (work, considered, scanned) = last.expect("retarget");
    CompositorCase {
        id: "compositor-retarget",
        kind: "Animation",
        status: "ok",
        tracks,
        properties: "opacity",
        workload: "retarget",
        work: snapshot(work, considered, scanned),
        retargets: Some(tracks),
        start_stop: None,
        steady_ms: super::summarize(&samples),
    }
}

fn measure_churn(document: DocumentId, tracks: usize) -> CompositorCase {
    let mut samples = Vec::new();
    let mut last = None;
    for iteration in 0..8 {
        let mut world = UiWorld::new();
        let ids = install_tracks(&mut world, document, tracks, PropertySet::Opacity);
        let mut scene = promote(&mut world, &ids);
        let mut stop = MutationQueue::new();
        for index in 1..=tracks {
            stop.stop_animation(AnimationId::new(index as u64).unwrap());
        }
        world.commit(stop).unwrap();
        let _ = world.take_system_work();
        let mut restart = MutationQueue::new();
        for (index, node) in ids.iter().enumerate() {
            restart.start_animation(opacity_spec((index + 1) as u64, *node, 0, 0.0));
        }
        world.commit(restart).unwrap();
        let _ = world.take_system_work();
        drain_due(&mut world, &mut scene, Duration::ZERO);
        let queries = world.presentation_values_cpu_sampled();
        let started = Instant::now();
        let frame = world.advance_animations(Duration::from_millis(STEADY_MS));
        let elapsed = started.elapsed();
        let work = merge_frame(&mut world, &mut scene, queries, &frame);
        assert!(quiet(work), "churn steady must stay quiet: {work:?}");
        last = Some((
            work,
            frame.animations_considered,
            frame.animation_deadlines_scanned,
        ));
        if iteration >= 2 {
            samples.push(elapsed.as_secs_f64() * 1_000.0);
        }
    }
    let (work, considered, scanned) = last.expect("churn");
    CompositorCase {
        id: "compositor-churn",
        kind: "Animation",
        status: "ok",
        tracks,
        properties: "opacity",
        workload: "churn",
        work: snapshot(work, considered, scanned),
        retargets: None,
        start_stop: Some(tracks),
        steady_ms: super::summarize(&samples),
    }
}

fn promote(world: &mut UiWorld, ids: &[nana_ui_runtime::StableNodeId]) -> UiScene {
    world.advance_animations(Duration::ZERO);
    let _ = world.take_system_work();
    let extracted = world.extract_nodes(ids);
    world.record_extract(&extracted);
    let mut scene = UiScene::new();
    scene.apply_delta(extracted, []);
    scene.apply_presentation(
        world.presentation_store(),
        LAYER_PROMOTE_HOLD,
        Some(world.motion_descriptors()),
    );
    scene
}

fn drain_due(world: &mut UiWorld, scene: &mut UiScene, now: Duration) {
    let _ = world.advance_animations(now);
    let _ = world.take_system_work();
    scene.apply_presentation(
        world.presentation_store(),
        now,
        Some(world.motion_descriptors()),
    );
}

fn merge_frame(
    world: &mut UiWorld,
    scene: &mut UiScene,
    queries_before: usize,
    frame: &nana_ui_runtime::AnimationFrame,
) -> MotionWorkCounters {
    let _ = frame;
    let drain = world.take_system_work();
    assert!(
        drain.is_empty(),
        "compositor steady drain must be empty: {drain:?}"
    );
    scene.apply_presentation(
        world.presentation_store(),
        Duration::from_millis(STEADY_MS),
        Some(world.motion_descriptors()),
    );
    let mut work = world.last_motion_frame_counters();
    work.presentation_values_cpu_sampled = world
        .presentation_values_cpu_sampled()
        .saturating_sub(queries_before);
    work.merge(scene.compositor_work_counters());
    work
}

fn quiet(work: MotionWorkCounters) -> bool {
    work.compositor_steady_is_quiet()
}

fn install_tracks(
    world: &mut UiWorld,
    document: DocumentId,
    tracks: usize,
    properties: PropertySet,
) -> Vec<nana_ui_runtime::StableNodeId> {
    let nodes = match properties {
        PropertySet::Mixed => tracks.div_ceil(2).max(1),
        _ => tracks.max(1),
    };
    let mut queue = MutationQueue::new();
    for index in 1..=nodes {
        let id = node(index);
        queue.create(id, document, NodeKind::Element { tag: "div".into() });
        if index > 1 {
            queue.insert(node(index / 2), id, None);
        }
    }
    let mut animation_id = 1u64;
    match properties {
        PropertySet::Transform => {
            for index in 1..=tracks {
                queue.start_animation(transform_spec(animation_id, node(index)));
                animation_id += 1;
            }
        }
        PropertySet::Opacity => {
            for index in 1..=tracks {
                queue.start_animation(opacity_spec(animation_id, node(index), 0, 0.0));
                animation_id += 1;
            }
        }
        PropertySet::Mixed => {
            for index in 1..=nodes {
                if animation_id <= tracks as u64 {
                    queue.start_animation(transform_spec(animation_id, node(index)));
                    animation_id += 1;
                }
                if animation_id <= tracks as u64 {
                    queue.start_animation(opacity_spec(animation_id, node(index), 0, 0.0));
                    animation_id += 1;
                }
            }
        }
    }
    world.commit(queue).unwrap();
    let _ = world.take_system_work();
    (1..=nodes).map(node).collect()
}

fn snapshot(work: MotionWorkCounters, considered: usize, scanned: usize) -> MotionWorkSnapshot {
    MotionWorkSnapshot {
        motion_tracks_active: work.motion_tracks_active,
        motion_tracks_cpu: work.motion_tracks_cpu,
        motion_tracks_compositor: work.motion_tracks_compositor,
        presentation_values_cpu_sampled: work.presentation_values_cpu_sampled,
        compositor_layers_active: work.compositor_layers_active,
        compositor_layers_promoted: work.compositor_layers_promoted,
        compositor_layers_demoted: work.compositor_layers_demoted,
        compositor_cache_bytes: work.compositor_cache_bytes,
        uiworld_mutations_from_animation: work.uiworld_mutations_from_animation,
        layout_nodes_from_animation: work.layout_nodes_from_animation,
        style_processed_from_animation: work.style_processed_from_animation,
        render_nodes_reextracted_from_animation: work.render_nodes_reextracted_from_animation,
        animations_considered: considered,
        animation_deadlines_scanned: scanned,
    }
}

fn opacity_spec(
    id: u64,
    target: nana_ui_runtime::StableNodeId,
    start_ms: u64,
    from: f32,
) -> AnimationSpec {
    AnimationSpec::new(
        AnimationId::new(id).unwrap(),
        target,
        Duration::from_millis(start_ms),
        Duration::from_millis(DURATION_MS),
        Duration::from_millis(16),
        Easing::Linear,
    )
    .with_property(AnimatableProperty::Opacity)
    .with_range(
        MotionValue::Scalar(from),
        MotionTo::Value(MotionValue::Scalar(1.0)),
    )
}

fn transform_spec(id: u64, target: nana_ui_runtime::StableNodeId) -> AnimationSpec {
    AnimationSpec::new(
        AnimationId::new(id).unwrap(),
        target,
        Duration::ZERO,
        Duration::from_millis(DURATION_MS),
        Duration::from_millis(16),
        Easing::Linear,
    )
    .with_property(AnimatableProperty::Transform)
    .with_range(
        MotionValue::Transform(PaintTransform::default()),
        MotionTo::Value(MotionValue::Transform(PaintTransform {
            a: 1.0,
            b: 0.0,
            c: 0.0,
            d: 1.0,
            e: 24.0,
            f: 0.0,
        })),
    )
}

fn node(index: usize) -> nana_ui_runtime::StableNodeId {
    nana_ui_runtime::StableNodeId::new(index as u64).unwrap()
}
