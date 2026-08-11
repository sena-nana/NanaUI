//! Phase 2: QuickJS-only Vue `runtime-core` probe.
//!
//! Mutually exclusive with `examples/vue-v8` — this binary never links V8.
//!
//! Phase 3 will replace the stub host renderer ops with Rust DOM + Blitz paint;
//! this example intentionally stops at JS execution + HostApiRegistry callbacks.

use std::env;
use std::time::{Duration, Instant};

use nana_js_engine::probe::{probe_host_registry, vue_runtime_probe_artifact};
use nana_js_engine::{HostValue, JsEngine};
use nana_js_quickjs::QuickJsEngine;
use nana_ui_vue::VueHost;

fn main() {
    nana_ui_vue::refuse_dual_js_engines!();

    let iterations = env::args()
        .nth(1)
        .and_then(|s| s.parse::<usize>().ok())
        .unwrap_or(1)
        .max(1);

    let mut colds = Vec::with_capacity(iterations);
    let mut invokes = Vec::with_capacity(iterations);
    let mut last_host = None;

    for _ in 0..iterations {
        let mut host = VueHost::new();
        let mut engine = QuickJsEngine::new();
        host.attach_engine(&mut engine).expect("attach vue host");

        let (api, state) = probe_host_registry();
        engine.register_host_api(&api).expect("register host api");

        let cold = Instant::now();
        engine
            .initialize(vue_runtime_probe_artifact())
            .expect("initialize vue probe on QuickJS");
        colds.push(cold.elapsed());

        let run = engine
            .resolve_function("__nanaProbe.run")
            .expect("resolve __nanaProbe.run");

        let invoke = Instant::now();
        let result = engine.invoke(run, &[]).expect("invoke probe");
        engine.run_microtasks().expect("microtasks");
        invokes.push(invoke.elapsed());

        let object = result.as_object().expect("object result");
        let guard = state.lock().expect("state");
        last_host = Some((
            object
                .get("ok")
                .and_then(HostValue::as_bool)
                .unwrap_or(false),
            object
                .get("count")
                .and_then(HostValue::as_f64)
                .unwrap_or(0.0),
            guard.create_element,
            guard.insert,
            guard.increment,
            guard.last_count,
        ));
        drop(guard);
        engine.shutdown();
    }

    let (ok, count, create_element, insert, increment, last_count) =
        last_host.expect("at least one iteration");

    println!("engine=quickjs");
    println!("ok={ok}");
    println!("count={count}");
    println!("host.createElement={create_element}");
    println!("host.insert={insert}");
    println!("host.increment={increment}");
    println!("host.lastCount={last_count}");
    println!("iterations={iterations}");
    println!("cold_start_ms_median={:.3}", median_ms(&colds));
    println!("invoke_ms_median={:.3}", median_ms(&invokes));
}

fn median_ms(samples: &[Duration]) -> f64 {
    let mut values: Vec<f64> = samples.iter().map(|d| d.as_secs_f64() * 1000.0).collect();
    values.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let mid = values.len() / 2;
    if values.len() % 2 == 0 {
        (values[mid - 1] + values[mid]) / 2.0
    } else {
        values[mid]
    }
}
