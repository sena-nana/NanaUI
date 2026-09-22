//! Scoped timing into a histogram.

use std::time::Instant;

use crate::metric::Metric;
use crate::runtime::metrics_enabled;

/// Records the elapsed nanoseconds into its histogram on drop. When metrics
/// are off it never reads the clock.
#[must_use = "a span measures until it is dropped"]
pub struct SpanGuard {
    metric: &'static Metric,
    start: Option<Instant>,
}

impl SpanGuard {
    #[inline]
    pub fn new(metric: &'static Metric) -> Self {
        Self {
            metric,
            start: metrics_enabled().then(Instant::now),
        }
    }

    /// Always measures, regardless of the global switch (for instances and
    /// tests).
    pub fn always(metric: &'static Metric) -> Self {
        Self {
            metric,
            start: Some(Instant::now()),
        }
    }
}

impl Drop for SpanGuard {
    #[inline]
    fn drop(&mut self) {
        if let Some(start) = self.start {
            self.metric
                .record(u64::try_from(start.elapsed().as_nanos()).unwrap_or(u64::MAX));
        }
    }
}
