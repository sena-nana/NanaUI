#[path = "runtime_benchmark/style_sharing.rs"]
mod style_sharing;
#[path = "runtime_benchmark/styled_mount.rs"]
mod styled_mount;

use std::{
    collections::BTreeMap,
    sync::Arc,
    time::{Duration, Instant},
};

use nana_ui_runtime::{
    AccessibilityRole, AccessibilityState, AnimationId, AnimationSpec, DocumentId, Easing,
    InteractionStyle, MutationQueue, NodeKind, NodeStyle, SemanticPaint, StableNodeId, SystemWork,
    TextContent, UiWorld,
};
use serde::Serialize;

const FRAME_BUDGET_MS: f64 = 16.67;
const SMALL_WARMUP: usize = 10;
const SMALL_ITERATIONS: usize = 60;
const SCALE_10K_WARMUP: usize = 5;
const SCALE_10K_ITERATIONS: usize = 20;
const CONSTRUCTION_WARMUP: usize = 2;
const CONSTRUCTION_ITERATIONS: usize = 8;
const CATALOG_ANIMATION_WARMUP: usize = 10;
const CATALOG_ANIMATION_ITERATIONS: usize = 40;
const CATALOG_ANIMATION_SCHEDULED: usize = 64;
const CATALOG_ANIMATION_ACTIVE: usize = 1;
/// Host ticks after settle used to count UI frames. Not a timer window.
const IDLE_FRAME_OBSERVE_TICKS: usize = 8;

#[derive(Serialize)]
struct Report {
    schema_version: u32,
    profile: &'static str,
    storage: &'static str,
    frame_budget_ms: f64,
    frame_budget_hz: u32,
    warmup_iterations: usize,
    iterations: usize,
    cases: Vec<Case>,
    /// Dedicated Issue #8 `animation` drain. Not the 5k tree incidental sample.
    catalog_animation: CatalogAnimationCase,
    notes: Vec<&'static str>,
}

#[derive(Serialize)]
struct Case {
    nodes: usize,
    kind: &'static str,
    warmup_iterations: usize,
    iterations: usize,
    enqueue_ms: Distribution,
    initial_commit_ms: Distribution,
    #[serde(skip_serializing_if = "Option::is_none")]
    initial_schedule_ms: Option<Distribution>,
    #[serde(skip_serializing_if = "Option::is_none")]
    initial_systems_ms: Option<Distribution>,
    #[serde(skip_serializing_if = "Option::is_none")]
    steady_reorder_commit_ms: Option<Distribution>,
    #[serde(skip_serializing_if = "Option::is_none")]
    steady_schedule_ms: Option<Distribution>,
    #[serde(skip_serializing_if = "Option::is_none")]
    steady_systems_ms: Option<Distribution>,
    local_paint_commit_ms: Distribution,
    #[serde(skip_serializing_if = "Option::is_none")]
    local_paint_schedule_ms: Option<Distribution>,
    #[serde(skip_serializing_if = "Option::is_none")]
    local_paint_systems_ms: Option<Distribution>,
    local_paint_work_nodes: usize,
    idle_schedule_ms: Distribution,
    /// UI frames scheduled after the StaticTree settled. Not `idle_schedule_ms`.
    #[serde(skip_serializing_if = "Option::is_none")]
    frames_after_idle: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    idle_animation_deadline_ms: Option<Distribution>,
    #[serde(skip_serializing_if = "Option::is_none")]
    scheduled_animation_deadline_ms: Option<Distribution>,
    #[serde(skip_serializing_if = "Option::is_none")]
    sparse_animation_sample_ms: Option<Distribution>,
    #[serde(skip_serializing_if = "Option::is_none")]
    scheduled_animations: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    due_animation_samples: Option<usize>,
    pointer_hover_transition_ms: Distribution,
    pointer_hover_work_nodes: usize,
    /// Single-node mutation drains at 5k (Issue #8 §3.2 remaining kinds).
    #[serde(skip_serializing_if = "Option::is_none")]
    single_node_mutations: Option<BTreeMap<&'static str, MutationDrain>>,
}

/// Catalog `animation` { active: 1, scheduled_idle: true } on its own UiWorld.
#[derive(Serialize)]
struct CatalogAnimationCase {
    id: &'static str,
    kind: &'static str,
    status: &'static str,
    active: usize,
    scheduled_idle: bool,
    scheduled_animations: usize,
    due_animation_samples: usize,
    work: AnimationAdvanceWork,
    idle_animation_deadline_ms: Distribution,
    scheduled_animation_deadline_ms: Distribution,
    sparse_animation_sample_ms: Distribution,
}

/// Sparse-advance observation from [`UiWorld::advance_animations`].
#[derive(Serialize, Clone, Copy)]
struct AnimationAdvanceWork {
    animation_deadlines_scanned: usize,
    animations_considered: usize,
}

#[derive(Serialize, Clone)]
struct MutationDrain {
    commit_ms: Distribution,
    schedule_ms: Distribution,
    systems_ms: Distribution,
}

#[derive(Serialize, Clone, Copy)]
struct Distribution {
    p50: f64,
    p95: f64,
    p99: f64,
    max: f64,
    frame_budget_ms: f64,
    frame_budget_misses: usize,
}

fn main() {
    let document = DocumentId::new(1).unwrap();
    if std::env::args().any(|arg| arg == "--profile-style-sharing") {
        style_sharing::run(document);
        return;
    }
    if std::env::args().any(|arg| arg == "--profile-styled-mount") {
        styled_mount::run(document);
        return;
    }
    if std::env::args().any(|arg| arg == "--profile-initial-systems") {
        profile_initial_systems(document);
        return;
    }
    prove_scheduled_ui_frames_are_not_hardcoded_zero(document);
    let mut cases = Vec::new();
    for nodes in [100, 500, 1_000, 5_000] {
        cases.push(bench_full(nodes, document, SMALL_WARMUP, SMALL_ITERATIONS));
    }
    cases.push(bench_full(
        10_000,
        document,
        SCALE_10K_WARMUP,
        SCALE_10K_ITERATIONS,
    ));
    cases.push(bench_construction(
        50_000,
        document,
        CONSTRUCTION_WARMUP,
        CONSTRUCTION_ITERATIONS,
    ));
    let report = Report {
        schema_version: 8,
        profile: if cfg!(debug_assertions) {
            "debug"
        } else {
            "release"
        },
        storage: "nana-ui-runtime node table",
        frame_budget_ms: FRAME_BUDGET_MS,
        frame_budget_hz: 60,
        warmup_iterations: SMALL_WARMUP,
        iterations: SMALL_ITERATIONS,
        cases,
        catalog_animation: bench_catalog_animation(document),
        notes: vec![
            "Generated by: cargo run --release --locked -p nana-ui-runtime --features benchmark --bin nana-runtime-benchmark -- --output perf/fixtures/nana-runtime-static-tree.json",
            "kind=full cases export frames_after_idle (§8.1). 50k is construction and omits the field.",
        ],
    };
    write_report(&report);
}

fn bench_full(nodes: usize, document: DocumentId, warmup: usize, iterations: usize) -> Case {
    let mut enqueue = Vec::with_capacity(iterations);
    let mut initial_commit = Vec::with_capacity(iterations);
    let mut initial_schedule = Vec::with_capacity(iterations);
    let mut initial_systems = Vec::with_capacity(iterations);
    let mut steady_commit = Vec::with_capacity(iterations);
    let mut steady_schedule = Vec::with_capacity(iterations);
    let mut steady_systems = Vec::with_capacity(iterations);
    let mut local_paint_commit = Vec::with_capacity(iterations);
    let mut local_paint_schedule = Vec::with_capacity(iterations);
    let mut local_paint_systems = Vec::with_capacity(iterations);
    let mut idle_schedule = Vec::with_capacity(iterations);
    let mut idle_animation_deadline = Vec::with_capacity(iterations);
    let mut scheduled_animation_deadline = Vec::with_capacity(iterations);
    let mut sparse_animation_sample = Vec::with_capacity(iterations);
    let mut pointer_hover_transition = Vec::with_capacity(iterations);
    let mut last_paint_work_nodes = 0;
    let mut last_hover_work_nodes = 0;
    let mut mutation_commits: BTreeMap<&'static str, Vec<Duration>> = BTreeMap::new();
    let mut mutation_schedules: BTreeMap<&'static str, Vec<Duration>> = BTreeMap::new();
    let mut mutation_systems: BTreeMap<&'static str, Vec<Duration>> = BTreeMap::new();
    let mut steady_world = UiWorld::new();
    steady_world
        .commit(tree_mutations(nodes, document))
        .unwrap();
    let initial_steady_work = steady_world.take_system_work();
    run_systems(&mut steady_world, document, &initial_steady_work);
    let mut interactive = MutationQueue::new();
    for target in [node(nodes.saturating_sub(1).max(1)), node(nodes)] {
        interactive.set_style(target, interactive_style(None));
    }
    steady_world.commit(interactive).unwrap();
    let work = steady_world.take_system_work();
    run_systems(&mut steady_world, document, &work);
    for iteration in 0..(warmup + iterations) {
        let started = Instant::now();
        let queue = tree_mutations(nodes, document);
        let enqueue_elapsed = started.elapsed();
        let started = Instant::now();
        let mut world = UiWorld::new();
        world.commit(queue).unwrap();
        let initial_commit_elapsed = started.elapsed();
        let started = Instant::now();
        let initial_work = world.take_system_work();
        let initial_schedule_elapsed = started.elapsed();
        let started = Instant::now();
        run_systems(&mut world, document, &initial_work);
        let initial_systems_elapsed = started.elapsed();
        let started = Instant::now();
        let deadline = world.next_animation_deadline();
        let idle_animation_deadline_elapsed = started.elapsed();
        assert_eq!(deadline, None);
        let mut animations = MutationQueue::new();
        for index in 1..=nodes {
            let due = index == 1;
            animations.start_animation(AnimationSpec::new(
                AnimationId::new(index as u64).unwrap(),
                node(index),
                if due {
                    Duration::ZERO
                } else {
                    Duration::from_secs(60)
                },
                if due {
                    Duration::from_millis(1)
                } else {
                    Duration::from_secs(1)
                },
                Duration::from_millis(16),
                Easing::Linear,
            ));
        }
        world.commit(animations).unwrap();
        let started = Instant::now();
        let deadline = world.next_animation_deadline();
        let scheduled_animation_deadline_elapsed = started.elapsed();
        assert_eq!(deadline, Some(Duration::ZERO));
        let started = Instant::now();
        let frame = world.advance_animations(Duration::from_millis(1));
        let sparse_animation_sample_elapsed = started.elapsed();
        assert_eq!(frame.samples.len(), 1);
        assert_eq!(frame.samples[0].target, node(1));
        assert!(frame.samples[0].finished);
        assert_eq!(frame.next_deadline, Some(Duration::from_secs(60)));

        let mut queue = MutationQueue::new();
        if iteration.is_multiple_of(2) {
            queue.insert(node(1), node(3), Some(node(2)));
        } else {
            queue.insert(node(1), node(2), Some(node(3)));
        }
        let started = Instant::now();
        steady_world.commit(queue).unwrap();
        let steady_commit_elapsed = started.elapsed();
        let started = Instant::now();
        let work = steady_world.take_system_work();
        let steady_schedule_elapsed = started.elapsed();
        let started = Instant::now();
        run_systems(&mut steady_world, document, &work);
        let steady_systems_elapsed = started.elapsed();

        let mut queue = MutationQueue::new();
        queue.set_style(
            node(nodes),
            interactive_style(Some(if iteration.is_multiple_of(2) {
                [0.2, 0.4, 0.8, 1.0]
            } else {
                [0.8, 0.4, 0.2, 1.0]
            })),
        );
        let started = Instant::now();
        steady_world.commit(queue).unwrap();
        let local_paint_commit_elapsed = started.elapsed();
        let started = Instant::now();
        let paint_work = steady_world.take_system_work();
        let local_paint_schedule_elapsed = started.elapsed();
        assert_eq!(paint_work.style.len(), 1);
        assert_eq!(paint_work.render_extraction.len(), 1);
        assert!(paint_work.layout_frontier_seeds.is_empty());
        assert!(paint_work.input_hit_test.is_empty());
        let started = Instant::now();
        run_systems(&mut steady_world, document, &paint_work);
        let local_paint_systems_elapsed = started.elapsed();
        let paint_work_nodes = paint_work.render_extraction.len();
        let started = Instant::now();
        let idle = steady_world.take_system_work();
        let idle_schedule_elapsed = started.elapsed();
        assert!(idle.is_empty());
        let hover_target = if iteration.is_multiple_of(2) {
            node(nodes)
        } else {
            node(nodes.saturating_sub(1).max(1))
        };
        let started = Instant::now();
        steady_world
            .set_pointer_hover(document, 1, Some(hover_target))
            .unwrap();
        let pointer_hover_transition_elapsed = started.elapsed();
        let hover_work = steady_world.take_system_work();
        assert!((1..=2).contains(&hover_work.style.len()));
        assert_eq!(hover_work.style, hover_work.render_extraction);
        let hover_work_nodes = hover_work.render_extraction.len();
        steady_world.resolve_styles(&hover_work.style).unwrap();
        let mutation_drains = (nodes == 5_000)
            .then(|| measure_single_node_mutations(&mut steady_world, document, nodes, iteration));
        if iteration >= warmup {
            enqueue.push(enqueue_elapsed);
            initial_commit.push(initial_commit_elapsed);
            initial_schedule.push(initial_schedule_elapsed);
            initial_systems.push(initial_systems_elapsed);
            steady_commit.push(steady_commit_elapsed);
            steady_schedule.push(steady_schedule_elapsed);
            steady_systems.push(steady_systems_elapsed);
            local_paint_commit.push(local_paint_commit_elapsed);
            local_paint_schedule.push(local_paint_schedule_elapsed);
            local_paint_systems.push(local_paint_systems_elapsed);
            idle_schedule.push(idle_schedule_elapsed);
            idle_animation_deadline.push(idle_animation_deadline_elapsed);
            scheduled_animation_deadline.push(scheduled_animation_deadline_elapsed);
            sparse_animation_sample.push(sparse_animation_sample_elapsed);
            pointer_hover_transition.push(pointer_hover_transition_elapsed);
            last_paint_work_nodes = paint_work_nodes;
            last_hover_work_nodes = hover_work_nodes;
        }
        if let Some(drains) = mutation_drains
            && iteration >= warmup
        {
            for (kind, drain) in drains {
                mutation_commits.entry(kind).or_default().push(drain.0);
                mutation_schedules.entry(kind).or_default().push(drain.1);
                mutation_systems.entry(kind).or_default().push(drain.2);
            }
        }
    }
    let single_node_mutations = (nodes == 5_000).then(|| {
        mutation_commits
            .keys()
            .copied()
            .map(|kind| {
                (
                    kind,
                    MutationDrain {
                        commit_ms: summarize(&mutation_commits[&kind]),
                        schedule_ms: summarize(&mutation_schedules[&kind]),
                        systems_ms: summarize(&mutation_systems[&kind]),
                    },
                )
            })
            .collect()
    });
    Case {
        nodes,
        kind: "full",
        warmup_iterations: warmup,
        iterations,
        enqueue_ms: summarize(&enqueue),
        initial_commit_ms: summarize(&initial_commit),
        initial_schedule_ms: Some(summarize(&initial_schedule)),
        initial_systems_ms: Some(summarize(&initial_systems)),
        steady_reorder_commit_ms: Some(summarize(&steady_commit)),
        steady_schedule_ms: Some(summarize(&steady_schedule)),
        steady_systems_ms: Some(summarize(&steady_systems)),
        local_paint_commit_ms: summarize(&local_paint_commit),
        local_paint_schedule_ms: Some(summarize(&local_paint_schedule)),
        local_paint_systems_ms: Some(summarize(&local_paint_systems)),
        local_paint_work_nodes: last_paint_work_nodes,
        idle_schedule_ms: summarize(&idle_schedule),
        frames_after_idle: Some(measure_frames_after_idle(nodes, document)),
        idle_animation_deadline_ms: Some(summarize(&idle_animation_deadline)),
        scheduled_animation_deadline_ms: Some(summarize(&scheduled_animation_deadline)),
        sparse_animation_sample_ms: Some(summarize(&sparse_animation_sample)),
        scheduled_animations: Some(nodes),
        due_animation_samples: Some(1),
        pointer_hover_transition_ms: summarize(&pointer_hover_transition),
        pointer_hover_work_nodes: last_hover_work_nodes,
        single_node_mutations,
    }
}

fn bench_construction(
    nodes: usize,
    document: DocumentId,
    warmup: usize,
    iterations: usize,
) -> Case {
    let mut enqueue = Vec::with_capacity(iterations);
    let mut initial_commit = Vec::with_capacity(iterations);
    let mut local_paint_commit = Vec::with_capacity(iterations);
    let mut idle_schedule = Vec::with_capacity(iterations);
    let mut pointer_hover_transition = Vec::with_capacity(iterations);
    let mut last_paint_work_nodes = 0;
    let mut last_hover_work_nodes = 0;
    let mut steady_world = UiWorld::new();
    steady_world
        .commit(tree_mutations(nodes, document))
        .unwrap();
    let _ = steady_world.take_system_work();
    let mut interactive = MutationQueue::new();
    for target in [node(nodes.saturating_sub(1).max(1)), node(nodes)] {
        interactive.set_style(target, interactive_style(None));
    }
    steady_world.commit(interactive).unwrap();
    let _ = steady_world.take_system_work();
    for iteration in 0..(warmup + iterations) {
        let started = Instant::now();
        let queue = tree_mutations(nodes, document);
        let enqueue_elapsed = started.elapsed();
        let started = Instant::now();
        let mut world = UiWorld::new();
        world.commit(queue).unwrap();
        let initial_commit_elapsed = started.elapsed();
        let initial_work = world.take_system_work();
        assert_eq!(initial_work.style.len(), nodes);
        assert!(world.take_system_work().is_empty());

        let mut queue = MutationQueue::new();
        queue.set_style(
            node(nodes),
            interactive_style(Some(if iteration.is_multiple_of(2) {
                [0.2, 0.4, 0.8, 1.0]
            } else {
                [0.8, 0.4, 0.2, 1.0]
            })),
        );
        let started = Instant::now();
        steady_world.commit(queue).unwrap();
        let local_paint_commit_elapsed = started.elapsed();
        let paint_work = steady_world.take_system_work();
        assert_eq!(paint_work.style.len(), 1);
        assert_eq!(paint_work.render_extraction.len(), 1);
        assert!(paint_work.layout_frontier_seeds.is_empty());
        let paint_work_nodes = paint_work.render_extraction.len();
        let started = Instant::now();
        let idle = steady_world.take_system_work();
        let idle_schedule_elapsed = started.elapsed();
        assert!(idle.is_empty());
        let hover_target = if iteration.is_multiple_of(2) {
            node(nodes)
        } else {
            node(nodes.saturating_sub(1).max(1))
        };
        let started = Instant::now();
        steady_world
            .set_pointer_hover(document, 1, Some(hover_target))
            .unwrap();
        let pointer_hover_transition_elapsed = started.elapsed();
        let hover_work = steady_world.take_system_work();
        assert!((1..=2).contains(&hover_work.style.len()));
        assert_eq!(hover_work.style, hover_work.render_extraction);
        let hover_work_nodes = hover_work.render_extraction.len();
        if iteration >= warmup {
            enqueue.push(enqueue_elapsed);
            initial_commit.push(initial_commit_elapsed);
            local_paint_commit.push(local_paint_commit_elapsed);
            idle_schedule.push(idle_schedule_elapsed);
            pointer_hover_transition.push(pointer_hover_transition_elapsed);
            last_paint_work_nodes = paint_work_nodes;
            last_hover_work_nodes = hover_work_nodes;
        }
    }
    Case {
        nodes,
        kind: "construction",
        warmup_iterations: warmup,
        iterations,
        enqueue_ms: summarize(&enqueue),
        initial_commit_ms: summarize(&initial_commit),
        initial_schedule_ms: None,
        initial_systems_ms: None,
        steady_reorder_commit_ms: None,
        steady_schedule_ms: None,
        steady_systems_ms: None,
        local_paint_commit_ms: summarize(&local_paint_commit),
        local_paint_schedule_ms: None,
        local_paint_systems_ms: None,
        local_paint_work_nodes: last_paint_work_nodes,
        idle_schedule_ms: summarize(&idle_schedule),
        frames_after_idle: None,
        idle_animation_deadline_ms: None,
        scheduled_animation_deadline_ms: None,
        sparse_animation_sample_ms: None,
        scheduled_animations: None,
        due_animation_samples: None,
        pointer_hover_transition_ms: summarize(&pointer_hover_transition),
        pointer_hover_work_nodes: last_hover_work_nodes,
        single_node_mutations: None,
    }
}

type TimedMutationWork = (Duration, Duration, Duration);

fn measure_single_node_mutations(
    world: &mut UiWorld,
    document: DocumentId,
    nodes: usize,
    iteration: usize,
) -> Vec<(&'static str, TimedMutationWork)> {
    let text_target = node((nodes / 2).max(2));
    let layout_target = node((nodes / 2 + 1).max(3));
    let visibility_target = node((nodes / 2 + 2).max(4));
    let transform_target = node((nodes / 2 + 3).max(5));
    let a11y_target = node((nodes / 2 + 4).max(6));
    let even = iteration.is_multiple_of(2);
    let mut drains = Vec::new();
    let mut text = MutationQueue::new();
    text.set_text(
        text_target,
        TextContent {
            value: if even { "nana" } else { "ui!!" }.into(),
        },
    );
    drains.push(("Text", drain_mutation(world, document, text)));

    let mut layout = MutationQueue::new();
    layout.set_style(
        layout_target,
        NodeStyle {
            layout: Arc::new(nana_ui_core::LayoutStyle {
                width: Some(nana_ui_core::LengthSpec::Px(if even {
                    120.0
                } else {
                    140.0
                })),
                ..Default::default()
            }),
            ..NodeStyle::default()
        },
    );
    drains.push(("LayoutStyle", drain_mutation(world, document, layout)));

    let mut visibility = MutationQueue::new();
    visibility.set_style(
        visibility_target,
        NodeStyle {
            layout: Arc::new(nana_ui_core::LayoutStyle {
                hidden: even,
                ..Default::default()
            }),
            ..NodeStyle::default()
        },
    );
    drains.push(("Visibility", drain_mutation(world, document, visibility)));

    let mut transform = MutationQueue::new();
    transform.set_style(
        transform_target,
        NodeStyle {
            layout: Arc::new(nana_ui_core::LayoutStyle {
                transforms: nana_ui_core::TransformStyle {
                    transform: Some(nana_ui_core::PaintTransform {
                        e: if even { 4.0 } else { 8.0 },
                        ..Default::default()
                    }),
                    ..Default::default()
                }
                .into(),
                ..Default::default()
            }),
            ..NodeStyle::default()
        },
    );
    drains.push(("Transform", drain_mutation(world, document, transform)));

    let mut accessibility = MutationQueue::new();
    accessibility.set_accessibility(
        a11y_target,
        AccessibilityState {
            role: AccessibilityRole::Generic,
            label: Some(Arc::from(if even { "alpha" } else { "beta" })),
            ..AccessibilityState::default()
        },
    );
    drains.push((
        "Accessibility",
        drain_mutation(world, document, accessibility),
    ));
    drains
}

fn drain_mutation(
    world: &mut UiWorld,
    document: DocumentId,
    queue: MutationQueue,
) -> TimedMutationWork {
    let started = Instant::now();
    world.commit(queue).unwrap();
    let commit = started.elapsed();
    let started = Instant::now();
    let work = world.take_system_work();
    let schedule = started.elapsed();
    let started = Instant::now();
    run_systems(world, document, &work);
    let systems = started.elapsed();
    (commit, schedule, systems)
}

fn interactive_style(background: Option<[f32; 4]>) -> NodeStyle {
    NodeStyle {
        layout: Arc::new(nana_ui_core::LayoutStyle {
            background,
            ..Default::default()
        }),
        interaction: InteractionStyle {
            hovered: SemanticPaint {
                background: Some(nana_ui_core::SemanticColorRole::Hover),
                ..SemanticPaint::default()
            },
            ..InteractionStyle::default()
        }
        .into(),
        ..NodeStyle::default()
    }
}

/// Isolated catalog `animation` drain. Does not share the 5k full-case world.
fn bench_catalog_animation(document: DocumentId) -> CatalogAnimationCase {
    let mut idle_deadline = Vec::with_capacity(CATALOG_ANIMATION_ITERATIONS);
    let mut scheduled_deadline = Vec::with_capacity(CATALOG_ANIMATION_ITERATIONS);
    let mut sparse_sample = Vec::with_capacity(CATALOG_ANIMATION_ITERATIONS);
    let mut last_advance = None;
    for iteration in 0..(CATALOG_ANIMATION_WARMUP + CATALOG_ANIMATION_ITERATIONS) {
        let mut world = UiWorld::new();
        world
            .commit(tree_mutations(CATALOG_ANIMATION_SCHEDULED, document))
            .unwrap();
        let _ = world.take_system_work();

        let started = Instant::now();
        let deadline = world.next_animation_deadline();
        let idle_elapsed = started.elapsed();
        assert_eq!(deadline, None);

        let mut animations = MutationQueue::new();
        for index in 1..=CATALOG_ANIMATION_SCHEDULED {
            let due = index == CATALOG_ANIMATION_ACTIVE;
            animations.start_animation(AnimationSpec::new(
                AnimationId::new(index as u64).unwrap(),
                node(index),
                if due {
                    Duration::ZERO
                } else {
                    Duration::from_secs(60)
                },
                if due {
                    Duration::from_millis(1)
                } else {
                    Duration::from_secs(1)
                },
                Duration::from_millis(16),
                Easing::Linear,
            ));
        }
        world.commit(animations).unwrap();

        let started = Instant::now();
        let deadline = world.next_animation_deadline();
        let scheduled_elapsed = started.elapsed();
        assert_eq!(deadline, Some(Duration::ZERO));

        let started = Instant::now();
        let frame = world.advance_animations(Duration::from_millis(1));
        let sample_elapsed = started.elapsed();
        assert_eq!(frame.samples.len(), CATALOG_ANIMATION_ACTIVE);
        assert_eq!(frame.samples[0].target, node(CATALOG_ANIMATION_ACTIVE));
        assert!(frame.samples[0].finished);
        assert_eq!(frame.next_deadline, Some(Duration::from_secs(60)));
        assert_eq!(frame.animation_deadlines_scanned, CATALOG_ANIMATION_ACTIVE);
        assert_eq!(frame.animations_considered, CATALOG_ANIMATION_ACTIVE);
        last_advance = Some(AnimationAdvanceWork {
            animation_deadlines_scanned: frame.animation_deadlines_scanned,
            animations_considered: frame.animations_considered,
        });

        if iteration >= CATALOG_ANIMATION_WARMUP {
            idle_deadline.push(idle_elapsed);
            scheduled_deadline.push(scheduled_elapsed);
            sparse_sample.push(sample_elapsed);
        }
    }
    CatalogAnimationCase {
        id: "animation",
        kind: "Animation",
        status: "ok",
        active: CATALOG_ANIMATION_ACTIVE,
        scheduled_idle: true,
        scheduled_animations: CATALOG_ANIMATION_SCHEDULED,
        due_animation_samples: CATALOG_ANIMATION_ACTIVE,
        work: last_advance.expect("catalog animation must observe a sparse advance"),
        idle_animation_deadline_ms: summarize(&idle_deadline),
        scheduled_animation_deadline_ms: summarize(&scheduled_deadline),
        sparse_animation_sample_ms: summarize(&sparse_sample),
    }
}

fn run_systems(world: &mut UiWorld, document: DocumentId, work: &SystemWork) {
    run_systems_observed(world, document, work, |_| {});
}

fn run_systems_observed(
    world: &mut UiWorld,
    document: DocumentId,
    work: &SystemWork,
    completed: impl FnMut(&'static str),
) {
    run_systems_with_style_resolver(world, document, work, completed, |world, ids| {
        world.resolve_styles(ids).unwrap();
    });
}

fn run_systems_with_style_resolver(
    world: &mut UiWorld,
    document: DocumentId,
    work: &SystemWork,
    mut completed: impl FnMut(&'static str),
    resolve_styles: impl FnOnce(&mut UiWorld, &[StableNodeId]),
) {
    resolve_styles(world, &work.style);
    completed("style");
    world.reconcile_focus(&work.focus_ime);
    completed("focus");
    let _ = world.project_accessibility_nodes(&work.accessibility);
    completed("accessibility");
    let layout_ids = work
        .layout_frontier_seeds
        .iter()
        .map(|seed| seed.node)
        .collect::<Vec<_>>();
    let _ = world.layout_inputs(&layout_ids).unwrap();
    completed("layout_inputs");
    // Mirror RuntimeDocument: patch the subtrees whose geometry changed and fall
    // back to a full rebuild only when the change is structural. Always
    // rebuilding the document here would measure a path the product no longer
    // takes.
    if !work.input_hit_test.is_empty()
        && !world.rebuild_hit_test_scoped(document, &work.input_hit_test)
    {
        world.rebuild_hit_test(document);
    }
    completed("hit_test");
    let _ = world.extract_nodes(&work.render_extraction);
    completed("extraction");
}

fn profile_initial_systems(document: DocumentId) {
    let mut report = BTreeMap::new();
    for nodes in [5_000, 10_000] {
        let mut samples: BTreeMap<&str, Vec<Duration>> = BTreeMap::new();
        for iteration in 0..70 {
            let mut world = UiWorld::new();
            world.commit(tree_mutations(nodes, document)).unwrap();
            let work = world.take_system_work();
            let mut started = Instant::now();
            let total_started = started;
            run_systems_observed(&mut world, document, &work, |stage| {
                let elapsed = started.elapsed();
                if iteration >= 10 {
                    samples.entry(stage).or_default().push(elapsed);
                }
                started = Instant::now();
            });
            if iteration >= 10 {
                samples
                    .entry("total")
                    .or_default()
                    .push(total_started.elapsed());
            }
        }
        report.insert(
            nodes,
            samples
                .into_iter()
                .map(|(stage, samples)| (stage, summarize(&samples)))
                .collect::<BTreeMap<_, _>>(),
        );
    }
    let json = serde_json::to_string_pretty(&report).unwrap();
    if let Some(path) = std::env::args().skip_while(|arg| arg != "--output").nth(1) {
        std::fs::write(path, json).unwrap();
    } else {
        println!("{json}");
    }
}

fn prove_scheduled_ui_frames_are_not_hardcoded_zero(document: DocumentId) {
    let mut world = UiWorld::new();
    world.commit(tree_mutations(4, document)).unwrap();
    let work = world.take_system_work();
    run_systems(&mut world, document, &work);
    assert_eq!(
        world.scheduled_ui_frames(IDLE_FRAME_OBSERVE_TICKS),
        0,
        "settled StaticTree must not keep scheduling UI frames"
    );

    let mut paint = MutationQueue::new();
    paint.set_style(node(4), interactive_style(Some([0.2, 0.4, 0.8, 1.0])));
    world.commit(paint).unwrap();
    assert_ne!(
        world.scheduled_ui_frames(IDLE_FRAME_OBSERVE_TICKS),
        0,
        "busy paint after settle must schedule at least one UI frame; refusing a hardcoded 0"
    );
}

fn measure_frames_after_idle(nodes: usize, document: DocumentId) -> usize {
    let mut world = UiWorld::new();
    world.commit(tree_mutations(nodes, document)).unwrap();
    let work = world.take_system_work();
    run_systems(&mut world, document, &work);
    world.scheduled_ui_frames(IDLE_FRAME_OBSERVE_TICKS)
}

fn tree_mutations(nodes: usize, document: DocumentId) -> MutationQueue {
    let mut queue = MutationQueue::new();
    for index in 1..=nodes {
        let id = node(index);
        queue.create(id, document, NodeKind::Element { tag: "div".into() });
        if index > 1 {
            queue.insert(node(index / 2), id, None);
        }
    }
    queue
}

fn node(value: usize) -> StableNodeId {
    StableNodeId::new(value as u64).unwrap()
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
            assert!(
                arguments.next().is_none(),
                "unexpected arguments after --output destination"
            );
            if let Some(parent) = path
                .parent()
                .filter(|parent| !parent.as_os_str().is_empty())
            {
                std::fs::create_dir_all(parent)
                    .expect("benchmark output directory must be writable");
            }
            std::fs::write(&path, format!("{json}\n"))
                .expect("benchmark report destination must be writable");
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
