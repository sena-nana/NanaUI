//! Event-loop stall watchdog.
//!
//! A host marks each event-loop callback with [`busy`]. A watchdog thread
//! polls the mark; a callback still running after [`STALL_THRESHOLD`] is a
//! stall — the platform stops delivering input and paints, and Windows soon
//! labels the window "Not Responding". The watchdog then records a
//! [`MAIN_THREAD_STALLED`] fault naming the callback, the share of one core
//! the thread burned meanwhile (a spin and a wait look different), and, on
//! Windows x64, a few stack samples of the stuck thread. A stall that goes on
//! is reported again at each [`FOLLOW_UPS`] mark; when the callback returns, a
//! [`MAIN_THREAD_RECOVERED`] event records the whole stall.
//!
//! Waiting for events between callbacks is idle, not a stall: only time
//! inside a callback counts. Nothing here allocates or locks on the watched
//! thread; a [`busy`] scope costs a thread-local read and two atomic stores.
//!
//! [`MAIN_THREAD_STALLED`]: crate::framework::diagnostics::MAIN_THREAD_STALLED
//! [`MAIN_THREAD_RECOVERED`]: crate::framework::diagnostics::MAIN_THREAD_RECOVERED

use std::cell::Cell;
use std::fmt::Write as _;
use std::marker::PhantomData;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicU8, AtomicU64, Ordering};
use std::time::{Duration, Instant};

use crate::Field;
use crate::framework::diagnostics::{MAIN_THREAD_RECOVERED, MAIN_THREAD_STALLED};

#[cfg(all(windows, target_arch = "x86_64"))]
pub(crate) mod windows;
#[cfg(all(windows, target_arch = "x86_64"))]
use windows as platform;

#[cfg(not(all(windows, target_arch = "x86_64")))]
mod platform {
    use std::time::Duration;

    pub(super) struct Thread;

    impl Thread {
        pub(super) fn current() -> Self {
            Self
        }

        pub(super) fn cpu_time(&self) -> Option<Duration> {
            None
        }

        pub(super) fn sample(&self, _frames: &mut Vec<u64>) -> bool {
            false
        }
    }

    pub(super) fn describe(frames: &[u64], out: &mut String) {
        for (index, pc) in frames.iter().enumerate() {
            use std::fmt::Write as _;
            let _ = writeln!(out, "  {index:2} 0x{pc:x}");
        }
    }
}

/// How long one callback may run before it is reported. Windows marks a
/// window "Not Responding" after five seconds without taking messages.
pub const STALL_THRESHOLD: Duration = Duration::from_secs(5);
/// Later reports of a stall that has not ended, measured from its start.
pub const FOLLOW_UPS: [Duration; 2] = [Duration::from_secs(30), Duration::from_secs(120)];

const POLL: Duration = Duration::from_millis(250);
/// Stack samples in the first report, apart by [`SAMPLE_GAP`]: identical
/// samples mean a wait, differing ones a loop.
const FIRST_SAMPLES: usize = 3;
const SAMPLE_GAP: Duration = Duration::from_millis(200);

/// The event-loop callback a [`busy`] scope covers. The code is stable: the
/// report names it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum Callback {
    /// Surfaces can be created (startup, resume).
    Resumed = 1,
    /// A proxy wake-up: program messages, tasks, completions.
    WakeUp = 2,
    /// A window event: input, resize, redraw.
    WindowEvent = 3,
    /// The loop is about to wait: frame scheduling.
    AboutToWait = 4,
}

impl Callback {
    fn name(code: u8) -> &'static str {
        match code {
            1 => "resumed",
            2 => "proxy_wake_up",
            3 => "window_event",
            4 => "about_to_wait",
            _ => "callback",
        }
    }
}

struct Watch {
    origin: Instant,
    /// Start of the outermost callback in progress, in ns since `origin`
    /// plus one; 0 while the thread is between callbacks.
    since: AtomicU64,
    callback: AtomicU8,
    thread: platform::Thread,
    threshold: Duration,
    poll: Duration,
    sample_gap: Duration,
}

impl Watch {
    fn now(&self) -> u64 {
        u64::try_from(self.origin.elapsed().as_nanos()).unwrap_or(u64::MAX)
    }
}

static WATCH: OnceLock<Watch> = OnceLock::new();

thread_local! {
    static WATCHED: Cell<bool> = const { Cell::new(false) };
    static DEPTH: Cell<u32> = const { Cell::new(0) };
}

/// Watch the calling thread, typically the event-loop thread, and start the
/// watchdog. One thread per process is watched; later calls do nothing.
pub fn watch_current_thread() {
    watch_current_thread_with_timings(STALL_THRESHOLD, POLL, SAMPLE_GAP);
}

#[doc(hidden)]
pub fn watch_current_thread_with_timings_for_test(
    threshold: Duration,
    poll: Duration,
    sample_gap: Duration,
) {
    watch_current_thread_with_timings(threshold, poll, sample_gap);
}

fn watch_current_thread_with_timings(threshold: Duration, poll: Duration, sample_gap: Duration) {
    let mut installed = false;
    let watch = WATCH.get_or_init(|| {
        installed = true;
        Watch {
            origin: Instant::now(),
            since: AtomicU64::new(0),
            callback: AtomicU8::new(0),
            thread: platform::Thread::current(),
            threshold,
            poll,
            sample_gap,
        }
    });
    if !installed {
        return;
    }
    WATCHED.with(|watched| watched.set(true));
    let _ = std::thread::Builder::new()
        .name("nana-stall-watchdog".into())
        .spawn(move || run(watch));
}

/// Mark one callback on the watched thread; the stall clock runs until the
/// returned scope drops. Nested scopes count as the outermost one. On any
/// other thread this does nothing.
#[must_use = "the callback is marked until the scope drops"]
pub fn busy(callback: Callback) -> BusyScope {
    if !WATCHED.with(Cell::get) {
        return BusyScope {
            outermost: false,
            counted: false,
            _thread: PhantomData,
        };
    }
    let outermost = DEPTH.with(|depth| {
        let current = depth.get();
        depth.set(current.saturating_add(1));
        current == 0
    });
    if outermost && let Some(watch) = WATCH.get() {
        watch.callback.store(callback as u8, Ordering::Relaxed);
        watch
            .since
            .store(watch.now().saturating_add(1), Ordering::Release);
    }
    BusyScope {
        outermost,
        counted: true,
        _thread: PhantomData,
    }
}

/// Returned by [`busy`]; ends the callback's mark when dropped.
pub struct BusyScope {
    outermost: bool,
    counted: bool,
    /// Bound to the thread whose depth it counts.
    _thread: PhantomData<*const ()>,
}

impl Drop for BusyScope {
    fn drop(&mut self) {
        if !self.counted {
            return;
        }
        DEPTH.with(|depth| depth.set(depth.get().saturating_sub(1)));
        if self.outermost
            && let Some(watch) = WATCH.get()
        {
            watch.since.store(0, Ordering::Release);
        }
    }
}

/// One callback the watchdog has seen running.
struct Stall {
    since: u64,
    /// When the watchdog first saw it, and the thread's CPU time then.
    seen_at: u64,
    cpu_at_seen: Option<Duration>,
    reports: usize,
}

impl Stall {
    fn begin(watch: &Watch, since: u64) -> Self {
        Self {
            since,
            seen_at: watch.now(),
            cpu_at_seen: watch.thread.cpu_time(),
            reports: 0,
        }
    }

    fn started(&self) -> u64 {
        self.since - 1
    }

    fn elapsed(&self, watch: &Watch) -> Duration {
        Duration::from_nanos(watch.now().saturating_sub(self.started()))
    }

    fn still_running(&self, watch: &Watch) -> bool {
        watch.since.load(Ordering::Acquire) == self.since
    }

    fn poll(&mut self, watch: &Watch) {
        let due = match self.reports {
            0 => watch.threshold,
            n => match FOLLOW_UPS.get(n - 1) {
                Some(due) => *due,
                None => return,
            },
        };
        if self.elapsed(watch) >= due {
            self.report(watch);
        }
    }

    fn report(&mut self, watch: &Watch) {
        self.reports += 1;
        let samples = if self.reports == 1 { FIRST_SAMPLES } else { 1 };
        let mut taken: Vec<Vec<u64>> = Vec::with_capacity(samples);
        for index in 0..samples {
            if index > 0 {
                std::thread::sleep(watch.sample_gap);
            }
            if !self.still_running(watch) {
                break;
            }
            let mut frames = Vec::new();
            if watch.thread.sample(&mut frames) {
                taken.push(frames);
            }
        }
        if !self.still_running(watch) {
            // It returned while being sampled; the recovery says how long.
            return;
        }
        let elapsed = self.elapsed(watch);
        let cpu_permille = self.cpu_permille(watch);
        let callback = Callback::name(watch.callback.load(Ordering::Relaxed));
        let mut text = format!(
            "event-loop thread inside {callback} for {:.1}s",
            elapsed.as_secs_f64()
        );
        if cpu_permille != u64::MAX {
            let _ = write!(
                text,
                ", using {}.{}% of one core",
                cpu_permille / 10,
                cpu_permille % 10
            );
        }
        if taken.is_empty() {
            text.push_str("; no stack sample on this platform");
        }
        for (index, frames) in taken.iter().enumerate() {
            let _ = write!(text, "\nsample {}:", index + 1);
            match taken[..index].iter().position(|earlier| earlier == frames) {
                Some(same) => {
                    let _ = write!(text, " same as sample {}", same + 1);
                }
                None => {
                    text.push('\n');
                    platform::describe(frames, &mut text);
                }
            }
        }
        let elapsed_ms = u64::try_from(elapsed.as_millis()).unwrap_or(u64::MAX);
        if crate::enabled(MAIN_THREAD_STALLED.severity) {
            crate::__private::fault(
                &MAIN_THREAD_STALLED,
                &[
                    Field::new("elapsed_ms", elapsed_ms),
                    Field::new("report", self.reports as u64),
                    Field::new("cpu_permille", cpu_permille),
                ],
                Some(text.into_boxed_str()),
            );
        }
        if self.reports == 1
            && let Some(diagnostics) = crate::global()
        {
            diagnostics.snapshot("stall");
        }
    }

    fn cpu_permille(&self, watch: &Watch) -> u64 {
        let (Some(before), Some(now)) = (self.cpu_at_seen, watch.thread.cpu_time()) else {
            return u64::MAX;
        };
        let wall = watch.now().saturating_sub(self.seen_at);
        if wall == 0 {
            return u64::MAX;
        }
        let used = u64::try_from(now.saturating_sub(before).as_nanos()).unwrap_or(u64::MAX);
        used.saturating_mul(1000) / wall
    }

    fn finish(&self, watch: &Watch) {
        if self.reports == 0 {
            return;
        }
        // The callback ended within one poll of now.
        let stalled_ms = u64::try_from(self.elapsed(watch).as_millis()).unwrap_or(u64::MAX);
        crate::event!(MAIN_THREAD_RECOVERED, stalled_ms = stalled_ms);
    }
}

fn run(watch: &'static Watch) {
    let mut current: Option<Stall> = None;
    loop {
        std::thread::sleep(watch.poll);
        let since = watch.since.load(Ordering::Acquire);
        if let Some(stall) = &mut current {
            if stall.since == since {
                stall.poll(watch);
                continue;
            }
            stall.finish(watch);
            current = None;
        }
        if since != 0 {
            current = Some(Stall::begin(watch, since));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scopes_mark_only_the_outermost_callback_on_the_watched_thread() {
        // Not watched: nothing is counted.
        let scope = busy(Callback::WindowEvent);
        assert!(!scope.counted);
        drop(scope);

        watch_current_thread();
        let watch = WATCH.get().expect("installed");
        assert_eq!(watch.since.load(Ordering::Acquire), 0);
        let outer = busy(Callback::WakeUp);
        let since = watch.since.load(Ordering::Acquire);
        assert_ne!(since, 0);
        assert_eq!(
            watch.callback.load(Ordering::Relaxed),
            Callback::WakeUp as u8
        );
        let inner = busy(Callback::WindowEvent);
        assert_eq!(watch.since.load(Ordering::Acquire), since);
        drop(inner);
        assert_eq!(watch.since.load(Ordering::Acquire), since);
        drop(outer);
        assert_eq!(watch.since.load(Ordering::Acquire), 0);

        // Another thread is never the watched one.
        std::thread::spawn(|| assert!(!busy(Callback::AboutToWait).counted))
            .join()
            .unwrap();
    }
}
