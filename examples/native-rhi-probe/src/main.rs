#![cfg_attr(not(target_os = "macos"), allow(dead_code))]

use std::path::PathBuf;
use std::time::Instant;

use serde::Serialize;

#[cfg(target_os = "macos")]
mod metal;
mod wgpu_backend;

const WIDTH: u32 = 900;
const HEIGHT: u32 = 640;
const WARMUP_ITERATIONS: usize = 20;
const ITERATIONS: usize = 120;
const PASS_COUNTS: [usize; 2] = [1, 32];

#[derive(Debug, Clone, Copy)]
struct Sample {
    encode_ms: f64,
    submit_wait_ms: f64,
}

impl Sample {
    fn total_ms(self) -> f64 {
        self.encode_ms + self.submit_wait_ms
    }
}

trait ProbeBackend {
    fn name(&self) -> &'static str;
    fn adapter_name(&self) -> String;
    fn sample(&mut self, pass_count: usize) -> Sample;
}

#[derive(Debug, Serialize)]
struct Report {
    schema_version: u32,
    profile: &'static str,
    workload: Workload,
    cases: Vec<CaseReport>,
}

#[derive(Debug, Serialize)]
struct Workload {
    description: &'static str,
    viewport: [u32; 2],
    warmup_iterations: usize,
    iterations: usize,
    pass_counts: [usize; 2],
}

#[derive(Debug, Serialize)]
struct CaseReport {
    backend: &'static str,
    adapter: String,
    pass_count: usize,
    encode_ms: Distribution,
    submit_wait_ms: Distribution,
    total_ms: Distribution,
}

#[derive(Debug, Serialize)]
struct Distribution {
    p50: f64,
    p95: f64,
    p99: f64,
    min: f64,
    max: f64,
}

fn main() {
    #[cfg(target_os = "macos")]
    run();

    #[cfg(not(target_os = "macos"))]
    panic!("native-rhi-probe currently requires macOS for the direct Metal lane");
}

#[cfg(target_os = "macos")]
fn run() {
    let mut wgpu = wgpu_backend::WgpuProbe::new(WIDTH, HEIGHT);
    let mut metal = metal::MetalProbe::new(WIDTH, HEIGHT);
    let mut cases = Vec::new();

    for pass_count in PASS_COUNTS {
        let (wgpu_samples, metal_samples) = run_counterbalanced(&mut wgpu, &mut metal, pass_count);
        cases.push(report_case(&wgpu, pass_count, &wgpu_samples));
        cases.push(report_case(&metal, pass_count, &metal_samples));
    }

    let report = Report {
        schema_version: 1,
        profile: if cfg!(debug_assertions) {
            "debug"
        } else {
            "release"
        },
        workload: Workload {
            description: "offscreen BGRA8 render-pass clear and blocking completion; no draw, text, resource upload, surface, or presentation",
            viewport: [WIDTH, HEIGHT],
            warmup_iterations: WARMUP_ITERATIONS,
            iterations: ITERATIONS,
            pass_counts: PASS_COUNTS,
        },
        cases,
    };
    write_report(&report);
}

#[cfg(target_os = "macos")]
fn run_counterbalanced(
    wgpu: &mut dyn ProbeBackend,
    metal: &mut dyn ProbeBackend,
    pass_count: usize,
) -> (Vec<Sample>, Vec<Sample>) {
    let mut wgpu_samples = Vec::with_capacity(ITERATIONS);
    let mut metal_samples = Vec::with_capacity(ITERATIONS);
    for iteration in 0..(WARMUP_ITERATIONS + ITERATIONS) {
        let (wgpu_sample, metal_sample) = if iteration.is_multiple_of(2) {
            (wgpu.sample(pass_count), metal.sample(pass_count))
        } else {
            let metal_sample = metal.sample(pass_count);
            let wgpu_sample = wgpu.sample(pass_count);
            (wgpu_sample, metal_sample)
        };
        if iteration >= WARMUP_ITERATIONS {
            wgpu_samples.push(wgpu_sample);
            metal_samples.push(metal_sample);
        }
    }
    (wgpu_samples, metal_samples)
}

fn report_case(backend: &dyn ProbeBackend, pass_count: usize, samples: &[Sample]) -> CaseReport {
    CaseReport {
        backend: backend.name(),
        adapter: backend.adapter_name(),
        pass_count,
        encode_ms: summarize(samples.iter().map(|sample| sample.encode_ms)),
        submit_wait_ms: summarize(samples.iter().map(|sample| sample.submit_wait_ms)),
        total_ms: summarize(samples.iter().copied().map(Sample::total_ms)),
    }
}

fn summarize(values: impl Iterator<Item = f64>) -> Distribution {
    let mut values = values.collect::<Vec<_>>();
    values.sort_by(f64::total_cmp);
    assert!(!values.is_empty(), "probe must contain samples");
    Distribution {
        p50: percentile(&values, 0.50),
        p95: percentile(&values, 0.95),
        p99: percentile(&values, 0.99),
        min: round(values[0]),
        max: round(values[values.len() - 1]),
    }
}

fn percentile(values: &[f64], percentile: f64) -> f64 {
    let index = ((values.len() - 1) as f64 * percentile).round() as usize;
    round(values[index])
}

fn round(value: f64) -> f64 {
    (value * 1_000_000.0).round() / 1_000_000.0
}

fn elapsed_ms(started: Instant) -> f64 {
    started.elapsed().as_secs_f64() * 1_000.0
}

fn write_report(report: &Report) {
    let json = serde_json::to_string_pretty(report).expect("probe report must serialize");
    let mut arguments = std::env::args_os().skip(1);
    match arguments.next() {
        None => println!("{json}"),
        Some(flag) if flag == "--output" => {
            let path = PathBuf::from(
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
                std::fs::create_dir_all(parent).expect("probe output directory must be writable");
            }
            std::fs::write(&path, format!("{json}\n"))
                .expect("probe report destination must be writable");
            println!("{}", path.display());
        }
        Some(argument) => panic!(
            "unsupported argument `{}`; expected --output <path>",
            argument.to_string_lossy()
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::{Sample, summarize};

    #[test]
    fn distribution_keeps_tail_samples() {
        let values = (1..=100).map(|value| value as f64).collect::<Vec<_>>();
        let distribution = summarize(values.into_iter());
        // The report uses the nearest index in the zero-based sorted samples.
        assert_eq!(distribution.p50, 51.0);
        assert_eq!(distribution.p95, 95.0);
        assert_eq!(distribution.p99, 99.0);
    }

    #[test]
    fn sample_total_contains_encode_and_completion() {
        let sample = Sample {
            encode_ms: 1.25,
            submit_wait_ms: 2.5,
        };
        assert_eq!(sample.total_ms(), 3.75);
    }
}
