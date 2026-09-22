//! Rate limiting for events that can recur every frame.

use std::sync::OnceLock;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

/// Lets at most one caller through per interval. Keep one per call site (or
/// per kind) in a `static`; racing threads never both win.
///
/// ```
/// use std::time::Duration;
/// use nana_diagnostics::Throttle;
/// static BUDGET_WARNING: Throttle = Throttle::new();
/// if BUDGET_WARNING.allow(Duration::from_secs(10)) {
///     // record the event
/// }
/// ```
pub struct Throttle {
    /// Milliseconds since the process epoch of the last pass, 0 = never.
    last_ms: AtomicU64,
}

impl Throttle {
    #[allow(clippy::new_without_default)]
    pub const fn new() -> Self {
        Self {
            last_ms: AtomicU64::new(0),
        }
    }

    pub fn allow(&self, interval: Duration) -> bool {
        static EPOCH: OnceLock<Instant> = OnceLock::new();
        // +1 so a pass in the first millisecond is not read as "never".
        let now_ms = EPOCH.get_or_init(Instant::now).elapsed().as_millis() as u64 + 1;
        self.allow_at(now_ms, interval.as_millis() as u64)
    }

    fn allow_at(&self, now_ms: u64, interval_ms: u64) -> bool {
        let last = self.last_ms.load(Ordering::Relaxed);
        if last != 0 && now_ms.saturating_sub(last) < interval_ms {
            return false;
        }
        self.last_ms
            .compare_exchange(last, now_ms, Ordering::Relaxed, Ordering::Relaxed)
            .is_ok()
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn one_pass_per_interval() {
        let throttle = super::Throttle::new();
        assert!(throttle.allow_at(1, 1000));
        assert!(!throttle.allow_at(2, 1000));
        assert!(!throttle.allow_at(1000, 1000));
        assert!(throttle.allow_at(1001, 1000));
        assert!(!throttle.allow_at(1500, 1000));
        assert!(throttle.allow_at(2500, 1000));
    }
}
