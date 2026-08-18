use std::sync::Arc;
use std::time::{Duration, Instant};

use nana_ui_runtime::{
    AnimationId, AnimationSpec, DocumentId, Easing, InteractionStyle, MutationQueue, NodeKind,
    NodeStyle, SemanticPaint, StableNodeId, SystemWork, UiWorld, WorkCounters,
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
    storage: &'static str,
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
    #[serde(skip_serializing_if = "Option::is_none")]
    initial_work: Option<WorkSnapshot>,
    #[serde(skip_serializing_if = "Option::is_none")]
    local_paint_work: Option<WorkSnapshot>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pointer_hover_work: Option<WorkSnapshot>,
}

/// Algorithm counts from [`SystemWork::counters`] / [`UiWorld::last_work_counters`].
/// GPU upload bytes are omitted: this binary does not observe renderer uploads.
#[derive(Serialize, Clone, Copy)]
struct WorkSnapshot {
    entities_total: usize,
    entities_changed: usize,
    entities_spawned: usize,
    entities_despawned: usize,
    style_processed: usize,
    text_shaped: usize,
    layout_nodes: usize,
    hit_test_candidates: usize,
    accessibility_nodes_updated: usize,
    render_nodes_extracted: usize,
    extracted_text_spans: usize,
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
    let document = DocumentId::new(1).unwrap();
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
        storage: "bevy_ecs 0.19.1 (default features disabled, std only)",
        frame_budget_ms: FRAME_BUDGET_MS,
        frame_budget_hz: 60,
        warmup_iterations: SMALL_WARMUP,
        iterations: SMALL_ITERATIONS,
        cases,
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
    let mut last_initial_work = None;
    let mut last_paint_work = None;
    let mut last_hover_work = None;
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
        let initial_snapshot = work_snapshot(&initial_work, &world);
        let started = Instant::now();
        let deadline = world.next_animation_deadline();
        let idle_animation_deadline_elapsed = started.elapsed();
        assert_eq!(deadline, None);
        let mut animations = MutationQueue::new();
        for index in 1..=nodes {
            let due = index == 1;
            animations.start_animation(AnimationSpec {
                id: AnimationId::new(index as u64).unwrap(),
                target: node(index),
                start: if due {
                    Duration::ZERO
                } else {
                    Duration::from_secs(60)
                },
                duration: if due {
                    Duration::from_millis(1)
                } else {
                    Duration::from_secs(1)
                },
                frame_interval: Duration::from_millis(16),
                easing: Easing::Linear,
            });
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
        assert!(paint_work.layout.is_empty());
        assert!(paint_work.input_hit_test.is_empty());
        let started = Instant::now();
        run_systems(&mut steady_world, document, &paint_work);
        let local_paint_systems_elapsed = started.elapsed();
        let paint_snapshot = work_snapshot(&paint_work, &steady_world);
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
        let hover_snapshot = work_snapshot(&hover_work, &steady_world);
        steady_world.resolve_styles(&hover_work.style).unwrap();
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
            last_initial_work = Some(initial_snapshot);
            last_paint_work = Some(paint_snapshot);
            last_hover_work = Some(hover_snapshot);
        }
    }
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
        local_paint_work_nodes: 1,
        idle_schedule_ms: summarize(&idle_schedule),
        idle_animation_deadline_ms: Some(summarize(&idle_animation_deadline)),
        scheduled_animation_deadline_ms: Some(summarize(&scheduled_animation_deadline)),
        sparse_animation_sample_ms: Some(summarize(&sparse_animation_sample)),
        scheduled_animations: Some(nodes),
        due_animation_samples: Some(1),
        pointer_hover_transition_ms: summarize(&pointer_hover_transition),
        pointer_hover_work_nodes: 2,
        initial_work: last_initial_work,
        local_paint_work: last_paint_work,
        pointer_hover_work: last_hover_work,
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
    let mut last_initial_work = None;
    let mut last_paint_work = None;
    let mut last_hover_work = None;
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
        let initial_snapshot = work_snapshot(&initial_work, &world);

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
        assert!(paint_work.layout.is_empty());
        let paint_snapshot = work_snapshot(&paint_work, &steady_world);
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
        let hover_snapshot = work_snapshot(&hover_work, &steady_world);
        if iteration >= warmup {
            enqueue.push(enqueue_elapsed);
            initial_commit.push(initial_commit_elapsed);
            local_paint_commit.push(local_paint_commit_elapsed);
            idle_schedule.push(idle_schedule_elapsed);
            pointer_hover_transition.push(pointer_hover_transition_elapsed);
            last_initial_work = Some(initial_snapshot);
            last_paint_work = Some(paint_snapshot);
            last_hover_work = Some(hover_snapshot);
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
        local_paint_work_nodes: 1,
        idle_schedule_ms: summarize(&idle_schedule),
        idle_animation_deadline_ms: None,
        scheduled_animation_deadline_ms: None,
        sparse_animation_sample_ms: None,
        scheduled_animations: None,
        due_animation_samples: None,
        pointer_hover_transition_ms: summarize(&pointer_hover_transition),
        pointer_hover_work_nodes: 2,
        initial_work: last_initial_work,
        local_paint_work: last_paint_work,
        pointer_hover_work: last_hover_work,
    }
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
        },
        ..NodeStyle::default()
    }
}

impl From<WorkCounters> for WorkSnapshot {
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
            accessibility_nodes_updated: counters.accessibility_nodes_updated,
            render_nodes_extracted: counters.render_nodes_extracted,
            extracted_text_spans: counters.extracted_text_spans,
        }
    }
}

fn work_snapshot(work: &SystemWork, world: &UiWorld) -> WorkSnapshot {
    let from_work = WorkSnapshot::from(work.counters());
    let last = world.last_work_counters();
    WorkSnapshot {
        render_nodes_extracted: if last.render_nodes_extracted > 0 {
            last.render_nodes_extracted
        } else {
            from_work.render_nodes_extracted
        },
        extracted_text_spans: last.extracted_text_spans,
        ..from_work
    }
}

fn run_systems(world: &mut UiWorld, document: DocumentId, work: &SystemWork) {
    world.resolve_styles(&work.style).unwrap();
    world.reconcile_focus(&work.focus_ime);
    let _ = world.project_accessibility_nodes(&work.accessibility);
    let _ = world.layout_inputs(&work.layout).unwrap();
    if !work.input_hit_test.is_empty() {
        world.rebuild_hit_test(document);
    }
    let _ = world.extract_nodes(&work.render_extraction);
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
