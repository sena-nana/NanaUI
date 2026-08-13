use std::time::{Duration, Instant};

use nana_ui_runtime::{
    AppContext, ContextPredicate, DocumentId, KeyContext, NodeKind, TextContent,
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
    let mut updates = Vec::with_capacity(ITERATIONS);
    let mut actions = Vec::with_capacity(ITERATIONS);
    for iteration in 0..(WARMUP_ITERATIONS + ITERATIONS) {
        let started = Instant::now();
        context
            .update(entity, |_view, cx| cx.emit(Increment))
            .unwrap();
        let update_elapsed = started.elapsed();
        let _ = context.world_mut().take_system_work();

        let started = Instant::now();
        context.dispatch_action(&action, &key_context).unwrap();
        let action_elapsed = started.elapsed();
        if iteration >= WARMUP_ITERATIONS {
            updates.push(update_elapsed);
            actions.push(action_elapsed);
        }
    }
    write_report(&Report {
        schema_version: 1,
        profile: if cfg!(debug_assertions) {
            "debug"
        } else {
            "release"
        },
        warmup_iterations: WARMUP_ITERATIONS,
        iterations: ITERATIONS,
        view_event_update_ms: summarize(&updates),
        action_dispatch_ms: summarize(&actions),
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
