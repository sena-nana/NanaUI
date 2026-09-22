//! Atomic metrics. High-frequency data (frame time, upload bytes, queue
//! depth) is aggregated here and sampled by the worker every
//! `metric_interval`; it never becomes one event per frame.
//!
//! Metrics are `static`s. The first touch links the metric into a lock-free
//! intrusive list the worker walks; after that a record is a few relaxed
//! atomic RMWs.

use std::sync::atomic::{AtomicBool, AtomicPtr, AtomicU64, Ordering};
use std::time::Duration;

use crate::schema::{Domain, MetricDescriptor, MetricKind};

/// Log2 buckets: bucket `i` holds values in `[2^(i-1), 2^i)`, bucket 0 holds 0.
pub const HISTOGRAM_BUCKETS: usize = 65;

pub struct Metric {
    pub descriptor: MetricDescriptor,
    registered: AtomicBool,
    next: AtomicPtr<Metric>,
    /// Counter total / gauge value (unused by histograms).
    value: AtomicU64,
    hist: Option<&'static HistogramCells>,
}

/// Histogram storage. Kept out of line so counters and gauges stay small.
pub struct HistogramCells {
    buckets: [AtomicU64; HISTOGRAM_BUCKETS],
    sum: AtomicU64,
    /// Interval min / max; the collector swaps them back to the identity.
    min: AtomicU64,
    max: AtomicU64,
}

impl HistogramCells {
    #[allow(clippy::new_without_default)]
    pub const fn new() -> Self {
        Self {
            buckets: [const { AtomicU64::new(0) }; HISTOGRAM_BUCKETS],
            sum: AtomicU64::new(0),
            min: AtomicU64::new(u64::MAX),
            max: AtomicU64::new(0),
        }
    }
}

static HEAD: AtomicPtr<Metric> = AtomicPtr::new(std::ptr::null_mut());

impl Metric {
    pub const fn counter(domain: Domain, id: u32, name: &'static str, unit: &'static str) -> Self {
        Self::with(domain, id, name, MetricKind::Counter, unit, None)
    }

    pub const fn gauge(domain: Domain, id: u32, name: &'static str, unit: &'static str) -> Self {
        Self::with(domain, id, name, MetricKind::Gauge, unit, None)
    }

    /// `cells` must be a dedicated `static HistogramCells` for this metric:
    ///
    /// ```
    /// use nana_diagnostics::{Domain, HistogramCells, Metric};
    /// static FRAME_NS_CELLS: HistogramCells = HistogramCells::new();
    /// pub static FRAME_NS: Metric =
    ///     Metric::histogram(Domain(0x0100), 1, "live.frame", "ns", &FRAME_NS_CELLS);
    /// ```
    pub const fn histogram(
        domain: Domain,
        id: u32,
        name: &'static str,
        unit: &'static str,
        cells: &'static HistogramCells,
    ) -> Self {
        Self::with(domain, id, name, MetricKind::Histogram, unit, Some(cells))
    }

    const fn with(
        domain: Domain,
        id: u32,
        name: &'static str,
        kind: MetricKind,
        unit: &'static str,
        hist: Option<&'static HistogramCells>,
    ) -> Self {
        Self {
            descriptor: MetricDescriptor {
                domain,
                id,
                name,
                kind,
                unit,
            },
            registered: AtomicBool::new(false),
            next: AtomicPtr::new(std::ptr::null_mut()),
            value: AtomicU64::new(0),
            hist,
        }
    }

    /// Counter: add. Gauge: set. Histogram: record one sample.
    ///
    /// Always records; the [`crate::metric!`] macro is what skips the call
    /// when diagnostics are off.
    #[inline]
    pub fn record(&'static self, value: u64) {
        self.register();
        match self.hist {
            Some(cells) => {
                cells.buckets[bucket_index(value)].fetch_add(1, Ordering::Relaxed);
                cells.sum.fetch_add(value, Ordering::Relaxed);
                // Read first: once the interval's extremes settle, neither
                // needs a read-modify-write.
                if value < cells.min.load(Ordering::Relaxed) {
                    cells.min.fetch_min(value, Ordering::Relaxed);
                }
                if value > cells.max.load(Ordering::Relaxed) {
                    cells.max.fetch_max(value, Ordering::Relaxed);
                }
            }
            None if self.descriptor.kind == MetricKind::Gauge => {
                self.value.store(value, Ordering::Relaxed);
            }
            None => {
                self.value.fetch_add(value, Ordering::Relaxed);
            }
        }
    }

    /// Counter total or gauge value (0 for histograms; snapshots carry
    /// their counts).
    pub fn value(&self) -> u64 {
        self.value.load(Ordering::Relaxed)
    }

    #[inline(always)]
    fn register(&'static self) {
        if self.registered.load(Ordering::Relaxed) {
            return;
        }
        self.register_slow();
    }

    #[cold]
    fn register_slow(&'static self) {
        if self.registered.swap(true, Ordering::AcqRel) {
            return;
        }
        let me = self as *const Metric as *mut Metric;
        let mut head = HEAD.load(Ordering::Acquire);
        loop {
            self.next.store(head, Ordering::Relaxed);
            match HEAD.compare_exchange_weak(head, me, Ordering::AcqRel, Ordering::Acquire) {
                Ok(_) => return,
                Err(current) => head = current,
            }
        }
    }

    /// Take this interval's histogram samples. `None` for counters and gauges.
    pub(crate) fn take_histogram(&self) -> Option<HistogramSample> {
        let cells = self.hist?;
        let mut buckets = Vec::new();
        let mut count = 0u64;
        for (index, bucket) in cells.buckets.iter().enumerate() {
            let taken = bucket.swap(0, Ordering::Relaxed);
            if taken != 0 {
                buckets.push((index as u8, taken));
                count += taken;
            }
        }
        let sum = cells.sum.swap(0, Ordering::Relaxed);
        let min = cells.min.swap(u64::MAX, Ordering::Relaxed);
        let max = cells.max.swap(0, Ordering::Relaxed);
        Some(HistogramSample {
            count,
            sum,
            min: if count == 0 { 0 } else { min },
            max,
            buckets,
        })
    }
}

/// Every metric touched so far in this process.
pub(crate) fn registered() -> impl Iterator<Item = &'static Metric> {
    let mut cursor = HEAD.load(Ordering::Acquire);
    std::iter::from_fn(move || {
        // SAFETY: only `&'static Metric`s are ever linked, and never unlinked.
        let metric = unsafe { cursor.as_ref() }?;
        cursor = metric.next.load(Ordering::Acquire);
        Some(metric)
    })
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct HistogramSample {
    pub count: u64,
    pub sum: u64,
    pub min: u64,
    pub max: u64,
    /// Non-empty `(bucket, count)` pairs.
    pub buckets: Vec<(u8, u64)>,
}

#[inline(always)]
pub fn bucket_index(value: u64) -> usize {
    (u64::BITS - value.leading_zeros()) as usize
}

/// Inclusive lower bound of a bucket.
pub fn bucket_floor(index: u8) -> u64 {
    match index {
        0 => 0,
        i => 1u64.checked_shl(u32::from(i) - 1).unwrap_or(u64::MAX),
    }
}

/// Conversion into a metric sample. [`Duration`] records nanoseconds.
pub trait MetricValue {
    fn to_metric(self) -> u64;
}

macro_rules! metric_value {
    ($($ty:ty),*) => {$(
        impl MetricValue for $ty {
            #[inline(always)]
            fn to_metric(self) -> u64 {
                self as u64
            }
        }
    )*};
}
metric_value!(u8, u16, u32, u64, usize);

impl MetricValue for Duration {
    #[inline(always)]
    fn to_metric(self) -> u64 {
        crate::record::saturating_ns(self)
    }
}

impl MetricValue for bool {
    #[inline(always)]
    fn to_metric(self) -> u64 {
        self as u64
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    static CELLS: HistogramCells = HistogramCells::new();
    static HIST: Metric = Metric::histogram(Domain(0x7F01), 1, "test.hist", "ns", &CELLS);
    static COUNTER: Metric = Metric::counter(Domain(0x7F01), 2, "test.counter", "count");
    static GAUGE: Metric = Metric::gauge(Domain(0x7F01), 3, "test.gauge", "count");

    #[test]
    fn buckets_are_log2() {
        assert_eq!(bucket_index(0), 0);
        assert_eq!(bucket_index(1), 1);
        assert_eq!(bucket_index(2), 2);
        assert_eq!(bucket_index(3), 2);
        assert_eq!(bucket_index(4), 3);
        assert_eq!(bucket_index(u64::MAX), 64);
        assert_eq!(bucket_floor(3), 4);
    }

    #[test]
    fn histogram_interval_resets_on_take() {
        for value in [1, 3, 1000] {
            HIST.record(value);
        }
        let sample = HIST.take_histogram().unwrap();
        assert_eq!(sample.count, 3);
        assert_eq!(sample.sum, 1004);
        assert_eq!((sample.min, sample.max), (1, 1000));
        assert_eq!(sample.buckets, [(1, 1), (2, 1), (10, 1)]);
        let empty = HIST.take_histogram().unwrap();
        assert_eq!(empty, HistogramSample::default());
    }

    #[test]
    fn counters_add_gauges_set_and_both_register_once() {
        COUNTER.record(2);
        COUNTER.record(3);
        GAUGE.record(7);
        GAUGE.record(4);
        assert_eq!(COUNTER.value(), 5);
        assert_eq!(GAUGE.value(), 4);
        let names: Vec<_> = registered().map(|m| m.descriptor.name).collect();
        assert_eq!(names.iter().filter(|n| **n == "test.counter").count(), 1);
        assert!(names.contains(&"test.gauge"));
    }
}
