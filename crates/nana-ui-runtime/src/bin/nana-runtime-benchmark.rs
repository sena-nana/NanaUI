use std::sync::Arc;
use std::time::{Duration, Instant};

use nana_ui_runtime::{
    AnimationId, AnimationSpec, DocumentId, Easing, MutationQueue, NodeKind, NodeStyle,
    StableNodeId, UiWorld,
};
use serde::Serialize;

const WARMUP_ITERATIONS: usize = 10;
const ITERATIONS: usize = 60;
const NODE_COUNTS: [usize; 4] = [100, 500, 1_000, 5_000];

#[derive(Serialize)]
struct Report {
    schema_version: u32,
    profile: &'static str,
    storage: &'static str,
    warmup_iterations: usize,
    iterations: usize,
    cases: Vec<Case>,
}

#[derive(Serialize)]
struct Case {
    nodes: usize,
    enqueue_ms: Distribution,
    initial_commit_ms: Distribution,
    initial_schedule_ms: Distribution,
    initial_systems_ms: Distribution,
    steady_reorder_commit_ms: Distribution,
    steady_schedule_ms: Distribution,
    steady_systems_ms: Distribution,
    local_paint_commit_ms: Distribution,
    local_paint_schedule_ms: Distribution,
    local_paint_systems_ms: Distribution,
    local_paint_work_nodes: usize,
    idle_schedule_ms: Distribution,
    idle_animation_deadline_ms: Distribution,
    active_animation_sample_ms: Distribution,
    active_animation_samples: usize,
}

#[derive(Serialize)]
struct Distribution {
    p50: f64,
    p95: f64,
    p99: f64,
}

fn main() {
    let document = DocumentId::new(1).unwrap();
    let mut cases = Vec::new();
    for nodes in NODE_COUNTS {
        let mut enqueue = Vec::with_capacity(ITERATIONS);
        let mut initial_commit = Vec::with_capacity(ITERATIONS);
        let mut initial_schedule = Vec::with_capacity(ITERATIONS);
        let mut initial_systems = Vec::with_capacity(ITERATIONS);
        let mut steady_commit = Vec::with_capacity(ITERATIONS);
        let mut steady_schedule = Vec::with_capacity(ITERATIONS);
        let mut steady_systems = Vec::with_capacity(ITERATIONS);
        let mut local_paint_commit = Vec::with_capacity(ITERATIONS);
        let mut local_paint_schedule = Vec::with_capacity(ITERATIONS);
        let mut local_paint_systems = Vec::with_capacity(ITERATIONS);
        let mut idle_schedule = Vec::with_capacity(ITERATIONS);
        let mut idle_animation_deadline = Vec::with_capacity(ITERATIONS);
        let mut active_animation_sample = Vec::with_capacity(ITERATIONS);
        let mut steady_world = UiWorld::new();
        steady_world
            .commit(tree_mutations(nodes, document))
            .unwrap();
        let initial_steady_work = steady_world.take_system_work();
        run_systems(&mut steady_world, document, &initial_steady_work);
        for iteration in 0..(WARMUP_ITERATIONS + ITERATIONS) {
            let started = Instant::now();
            let queue = tree_mutations(nodes, document);
            let enqueue_elapsed = started.elapsed();
            let started = Instant::now();
            let mut world = UiWorld::new();
            world.commit(queue).unwrap();
            let initial_commit_elapsed = started.elapsed();
            let started = Instant::now();
            let work = world.take_system_work();
            let initial_schedule_elapsed = started.elapsed();
            let started = Instant::now();
            run_systems(&mut world, document, &work);
            let initial_systems_elapsed = started.elapsed();
            let started = Instant::now();
            let deadline = world.next_animation_deadline();
            let idle_animation_deadline_elapsed = started.elapsed();
            assert_eq!(deadline, None);
            let mut animation = MutationQueue::new();
            animation.start_animation(AnimationSpec {
                id: AnimationId::new(1).unwrap(),
                target: node(nodes),
                start: Duration::ZERO,
                duration: Duration::from_secs(1),
                frame_interval: Duration::from_millis(16),
                easing: Easing::Linear,
            });
            world.commit(animation).unwrap();
            let started = Instant::now();
            let frame = world.advance_animations(Duration::from_millis(500));
            let active_animation_sample_elapsed = started.elapsed();
            assert_eq!(frame.samples.len(), 1);
            assert_eq!(frame.samples[0].target, node(nodes));

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
                NodeStyle {
                    layout: Arc::new(nana_ui_core::LayoutStyle {
                        background: Some(if iteration.is_multiple_of(2) {
                            [0.2, 0.4, 0.8, 1.0]
                        } else {
                            [0.8, 0.4, 0.2, 1.0]
                        }),
                        ..Default::default()
                    }),
                    foreground: None,
                },
            );
            let started = Instant::now();
            steady_world.commit(queue).unwrap();
            let local_paint_commit_elapsed = started.elapsed();
            let started = Instant::now();
            let work = steady_world.take_system_work();
            let local_paint_schedule_elapsed = started.elapsed();
            assert_eq!(work.style.len(), 1);
            assert_eq!(work.render_extraction.len(), 1);
            assert!(work.layout.is_empty());
            assert!(work.input_hit_test.is_empty());
            let started = Instant::now();
            run_systems(&mut steady_world, document, &work);
            let local_paint_systems_elapsed = started.elapsed();
            let started = Instant::now();
            let idle = steady_world.take_system_work();
            let idle_schedule_elapsed = started.elapsed();
            assert!(idle.is_empty());
            if iteration >= WARMUP_ITERATIONS {
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
                active_animation_sample.push(active_animation_sample_elapsed);
            }
        }
        cases.push(Case {
            nodes,
            enqueue_ms: summarize(&enqueue),
            initial_commit_ms: summarize(&initial_commit),
            initial_schedule_ms: summarize(&initial_schedule),
            initial_systems_ms: summarize(&initial_systems),
            steady_reorder_commit_ms: summarize(&steady_commit),
            steady_schedule_ms: summarize(&steady_schedule),
            steady_systems_ms: summarize(&steady_systems),
            local_paint_commit_ms: summarize(&local_paint_commit),
            local_paint_schedule_ms: summarize(&local_paint_schedule),
            local_paint_systems_ms: summarize(&local_paint_systems),
            local_paint_work_nodes: 1,
            idle_schedule_ms: summarize(&idle_schedule),
            idle_animation_deadline_ms: summarize(&idle_animation_deadline),
            active_animation_sample_ms: summarize(&active_animation_sample),
            active_animation_samples: 1,
        });
    }
    let report = Report {
        schema_version: 4,
        profile: if cfg!(debug_assertions) {
            "debug"
        } else {
            "release"
        },
        storage: "bevy_ecs 0.19.1 (default features disabled, std only)",
        warmup_iterations: WARMUP_ITERATIONS,
        iterations: ITERATIONS,
        cases,
    };
    write_report(&report);
}

fn run_systems(world: &mut UiWorld, document: DocumentId, work: &nana_ui_runtime::SystemWork) {
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
    }
}

fn percentile(values: &[f64], percentile: f64) -> f64 {
    let index = ((values.len() - 1) as f64 * percentile).round() as usize;
    (values[index] * 1_000.0).round() / 1_000.0
}
