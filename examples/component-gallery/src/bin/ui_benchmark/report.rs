use serde::Serialize;

#[derive(Debug, Serialize)]
pub struct BenchmarkReport {
    pub profile: &'static str,
    pub adapter: AdapterReport,
    pub iterations: usize,
    pub warmup_iterations: usize,
    pub viewport: [u32; 2],
    pub cases: Vec<CaseReport>,
}

#[derive(Debug, Serialize)]
pub struct AdapterReport {
    pub name: String,
    pub backend: String,
    pub device_type: String,
}

#[derive(Debug, Serialize)]
pub struct CaseReport {
    pub scenario: String,
    pub item_count: usize,
    pub view_construction_ms: Distribution,
    pub layout_diff_ms: Distribution,
    pub event_update_ms: Distribution,
    pub draw_cpu_ms: Distribution,
    pub cpu_total_ms: Distribution,
    pub gpu_submit_wait_ms: Distribution,
    pub total_ms: Distribution,
}

#[derive(Debug, Serialize)]
pub struct Distribution {
    pub median: f64,
    pub p95: f64,
    pub min: f64,
    pub max: f64,
}

#[derive(Debug, Clone, Copy)]
pub struct Sample {
    pub view_construction_ms: f64,
    pub layout_diff_ms: f64,
    pub event_update_ms: f64,
    pub draw_cpu_ms: f64,
    pub gpu_submit_wait_ms: f64,
}

impl Sample {
    fn cpu_total_ms(self) -> f64 {
        self.view_construction_ms + self.layout_diff_ms + self.event_update_ms + self.draw_cpu_ms
    }

    fn total_ms(self) -> f64 {
        self.cpu_total_ms() + self.gpu_submit_wait_ms
    }
}

impl CaseReport {
    pub fn from_samples(
        scenario: impl Into<String>,
        item_count: usize,
        samples: &[Sample],
    ) -> Self {
        Self {
            scenario: scenario.into(),
            item_count,
            view_construction_ms: summarize(
                samples.iter().map(|sample| sample.view_construction_ms),
            ),
            layout_diff_ms: summarize(samples.iter().map(|sample| sample.layout_diff_ms)),
            event_update_ms: summarize(samples.iter().map(|sample| sample.event_update_ms)),
            draw_cpu_ms: summarize(samples.iter().map(|sample| sample.draw_cpu_ms)),
            cpu_total_ms: summarize(samples.iter().copied().map(Sample::cpu_total_ms)),
            gpu_submit_wait_ms: summarize(samples.iter().map(|sample| sample.gpu_submit_wait_ms)),
            total_ms: summarize(samples.iter().copied().map(Sample::total_ms)),
        }
    }
}

fn summarize(values: impl Iterator<Item = f64>) -> Distribution {
    let mut values = values.collect::<Vec<_>>();
    values.sort_by(f64::total_cmp);
    assert!(!values.is_empty(), "benchmark must contain samples");
    let median = percentile(&values, 0.50);
    let p95 = percentile(&values, 0.95);

    Distribution {
        median: round(median),
        p95: round(p95),
        min: round(values[0]),
        max: round(values[values.len() - 1]),
    }
}

fn percentile(values: &[f64], percentile: f64) -> f64 {
    let index = ((values.len() - 1) as f64 * percentile).round() as usize;
    values[index]
}

fn round(value: f64) -> f64 {
    (value * 1_000.0).round() / 1_000.0
}

#[cfg(test)]
mod tests {
    use super::{CaseReport, Sample};

    #[test]
    fn case_report_preserves_tail_latency_and_total_cost() {
        let samples = [
            Sample {
                view_construction_ms: 1.0,
                layout_diff_ms: 2.0,
                event_update_ms: 3.0,
                draw_cpu_ms: 4.0,
                gpu_submit_wait_ms: 5.0,
            },
            Sample {
                view_construction_ms: 2.0,
                layout_diff_ms: 3.0,
                event_update_ms: 4.0,
                draw_cpu_ms: 5.0,
                gpu_submit_wait_ms: 6.0,
            },
            Sample {
                view_construction_ms: 10.0,
                layout_diff_ms: 11.0,
                event_update_ms: 12.0,
                draw_cpu_ms: 13.0,
                gpu_submit_wait_ms: 14.0,
            },
        ];
        let report = CaseReport::from_samples("list-100", 100, &samples);

        assert_eq!(report.scenario, "list-100");
        assert_eq!(report.item_count, 100);
        assert_eq!(report.view_construction_ms.median, 2.0);
        assert_eq!(report.view_construction_ms.p95, 10.0);
        assert_eq!(report.cpu_total_ms.median, 14.0);
        assert_eq!(report.cpu_total_ms.p95, 46.0);
        assert_eq!(report.total_ms.median, 20.0);
        assert_eq!(report.total_ms.p95, 60.0);
    }
}
