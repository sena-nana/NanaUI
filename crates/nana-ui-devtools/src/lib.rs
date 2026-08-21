//! Development-only diagnostics for the NanaUI Vue/JS and hosted rendering
//! stack. The crate stores bounded structured records and deliberately does not
//! provide product-facing UI.
//!
//! Optional `offscreen` / `agent` features add snapshot CPU readback and a
//! headless Agent session. Those paths are tooling-only and must not be wired
//! into product Surface present.

#[cfg(feature = "agent")]
pub mod agent;
#[cfg(feature = "offscreen")]
pub mod offscreen;

use std::collections::{BTreeMap, VecDeque};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use nana_js_engine::{
    HostCallObserver, HostCallTrace, JsDiagnosticEvent, JsDiagnosticLevel, JsDiagnosticSink,
};

const DEFAULT_EVENT_CAPACITY: usize = 2_048;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiagnosticKind {
    JsException,
    UnhandledPromiseRejection,
    VueWarning,
    VueError,
    HostCall,
    ResourceLifecycle,
    WindowLifecycle,
    Frame,
    DeviceLost,
    RenderError,
    Inspector,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiagnosticEvent {
    pub sequence: u64,
    pub elapsed_micros: u64,
    pub kind: DiagnosticKind,
    pub level: JsDiagnosticLevel,
    pub source: String,
    pub message: String,
    pub stack: Option<String>,
    pub fields: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FrameStatistics {
    pub presented_frames: u64,
    pub dropped_frames: u64,
    pub total_build_micros: u64,
    pub total_draw_micros: u64,
    pub total_present_micros: u64,
    pub slowest_frame_micros: u64,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DiagnosticsSnapshot {
    pub events: Vec<DiagnosticEvent>,
    pub resources: BTreeMap<String, usize>,
    pub frames: BTreeMap<u64, FrameStatistics>,
}

#[derive(Debug, Default)]
struct DiagnosticsState {
    next_sequence: u64,
    events: VecDeque<DiagnosticEvent>,
    resources: BTreeMap<String, usize>,
    frames: BTreeMap<u64, FrameStatistics>,
    last_presented: BTreeMap<u64, Instant>,
}

/// Cloneable recorder that can be connected to V8, VueHost and hosted render
/// loops without making any of those crates depend on this development crate.
#[derive(Debug, Clone)]
pub struct DevtoolsSession {
    started: Instant,
    capacity: usize,
    state: Arc<Mutex<DiagnosticsState>>,
}

impl Default for DevtoolsSession {
    fn default() -> Self {
        Self::new(DEFAULT_EVENT_CAPACITY)
    }
}

impl DevtoolsSession {
    pub fn new(capacity: usize) -> Self {
        Self {
            started: Instant::now(),
            capacity: capacity.max(1),
            state: Arc::new(Mutex::new(DiagnosticsState::default())),
        }
    }

    /// Sink accepted by JS backends and VueHost. V8 exception and Promise
    /// events retain their stack while Vue reports arrive through the same path.
    pub fn js_sink(&self) -> JsDiagnosticSink {
        let session = self.clone();
        Arc::new(move |event| session.record_js(event))
    }

    /// Observer accepted by `HostApiRegistry`. It intentionally records only
    /// method name, status and timing, never call arguments or results.
    pub fn host_call_observer(&self) -> HostCallObserver {
        let session = self.clone();
        Arc::new(move |trace| session.record_host_call(trace))
    }

    pub fn record_js(&self, event: JsDiagnosticEvent) {
        let kind = match event.source.as_str() {
            "v8.promise" => DiagnosticKind::UnhandledPromiseRejection,
            "vue.warn" => DiagnosticKind::VueWarning,
            "vue.error" => DiagnosticKind::VueError,
            "nana.resource" => DiagnosticKind::ResourceLifecycle,
            "nana.window" => DiagnosticKind::WindowLifecycle,
            "wgpu.device" => DiagnosticKind::DeviceLost,
            "wgpu.render" => DiagnosticKind::RenderError,
            _ => DiagnosticKind::JsException,
        };
        self.push(
            kind,
            event.level,
            event.source,
            event.message,
            event.stack,
            BTreeMap::new(),
        );
    }

    pub fn record_host_call(&self, trace: HostCallTrace) {
        let fields = [
            ("asynchronous".into(), trace.asynchronous.to_string()),
            ("pending".into(), trace.pending.to_string()),
            ("succeeded".into(), trace.succeeded.to_string()),
            ("durationMicros".into(), trace.duration_micros.to_string()),
        ]
        .into_iter()
        .collect();
        self.push(
            DiagnosticKind::HostCall,
            if trace.succeeded {
                JsDiagnosticLevel::Info
            } else {
                JsDiagnosticLevel::Error
            },
            "host.bridge",
            trace.name,
            None,
            fields,
        );
    }

    pub fn set_resource_count(&self, kind: impl Into<String>, count: usize) {
        if let Ok(mut state) = self.state.lock() {
            state.resources.insert(kind.into(), count);
        }
    }

    pub fn set_resource_counts(
        &self,
        namespace: &str,
        counts: impl IntoIterator<Item = (impl AsRef<str>, usize)>,
    ) {
        if let Ok(mut state) = self.state.lock() {
            for (kind, count) in counts {
                state
                    .resources
                    .insert(format!("{namespace}.{}", kind.as_ref()), count);
            }
        }
    }

    pub fn record_resource_lifecycle(&self, resource_kind: &str, action: &str, id: impl ToString) {
        self.record_lifecycle(DiagnosticKind::ResourceLifecycle, resource_kind, action, id);
    }

    pub fn record_window_lifecycle(&self, action: &str, id: impl ToString) {
        self.record_lifecycle(DiagnosticKind::WindowLifecycle, "window", action, id);
    }

    fn record_lifecycle(
        &self,
        kind: DiagnosticKind,
        resource_kind: &str,
        action: &str,
        id: impl ToString,
    ) {
        let fields = [
            ("resourceKind".into(), resource_kind.to_owned()),
            ("action".into(), action.to_owned()),
            ("id".into(), id.to_string()),
        ]
        .into_iter()
        .collect();
        self.push(
            kind,
            JsDiagnosticLevel::Info,
            "nana.lifecycle",
            action,
            None,
            fields,
        );
    }

    /// Record one completed frame. Dropped-frame estimation is based on the
    /// actual interval between presents and the caller-provided display budget.
    pub fn record_frame(
        &self,
        window_id: u64,
        target_interval: Duration,
        build: Duration,
        draw: Duration,
        present: Duration,
    ) {
        let now = Instant::now();
        let total = build.saturating_add(draw).saturating_add(present);
        let to_us = |value: Duration| value.as_micros().min(u64::MAX as u128) as u64;
        let (dropped, interval) = if let Ok(mut state) = self.state.lock() {
            let interval = state
                .last_presented
                .insert(window_id, now)
                .map(|last| now.duration_since(last));
            let dropped = interval
                .filter(|_| !target_interval.is_zero())
                .map(|elapsed| elapsed.as_nanos() / target_interval.as_nanos())
                .unwrap_or(1)
                .saturating_sub(1)
                .min(u64::MAX as u128) as u64;
            let stats = state.frames.entry(window_id).or_default();
            stats.presented_frames = stats.presented_frames.saturating_add(1);
            stats.dropped_frames = stats.dropped_frames.saturating_add(dropped);
            stats.total_build_micros = stats.total_build_micros.saturating_add(to_us(build));
            stats.total_draw_micros = stats.total_draw_micros.saturating_add(to_us(draw));
            stats.total_present_micros = stats.total_present_micros.saturating_add(to_us(present));
            stats.slowest_frame_micros = stats.slowest_frame_micros.max(to_us(total));
            (dropped, interval)
        } else {
            return;
        };
        let fields = [
            ("windowId".into(), window_id.to_string()),
            ("buildMicros".into(), to_us(build).to_string()),
            ("drawMicros".into(), to_us(draw).to_string()),
            ("presentMicros".into(), to_us(present).to_string()),
            (
                "intervalMicros".into(),
                interval.map(to_us).unwrap_or(0).to_string(),
            ),
            ("dropped".into(), dropped.to_string()),
        ]
        .into_iter()
        .collect();
        self.push(
            DiagnosticKind::Frame,
            JsDiagnosticLevel::Info,
            "nana.frame",
            "presented",
            None,
            fields,
        );
    }

    pub fn record_device_lost(&self, message: impl Into<String>) {
        self.push(
            DiagnosticKind::DeviceLost,
            JsDiagnosticLevel::Error,
            "wgpu.device",
            message,
            None,
            BTreeMap::new(),
        );
    }

    pub fn record_render_error(&self, message: impl Into<String>) {
        self.push(
            DiagnosticKind::RenderError,
            JsDiagnosticLevel::Error,
            "wgpu.render",
            message,
            None,
            BTreeMap::new(),
        );
    }

    pub fn record_inspector_transport(&self, direction: &str, byte_len: usize) {
        let fields = [
            ("direction".into(), direction.to_owned()),
            ("bytes".into(), byte_len.to_string()),
        ]
        .into_iter()
        .collect();
        self.push(
            DiagnosticKind::Inspector,
            JsDiagnosticLevel::Info,
            "v8.inspector",
            "protocol",
            None,
            fields,
        );
    }

    pub fn snapshot(&self) -> DiagnosticsSnapshot {
        let Ok(state) = self.state.lock() else {
            return DiagnosticsSnapshot::default();
        };
        DiagnosticsSnapshot {
            events: state.events.iter().cloned().collect(),
            resources: state.resources.clone(),
            frames: state.frames.clone(),
        }
    }

    pub fn drain_events(&self) -> Vec<DiagnosticEvent> {
        let Ok(mut state) = self.state.lock() else {
            return Vec::new();
        };
        state.events.drain(..).collect()
    }

    fn push(
        &self,
        kind: DiagnosticKind,
        level: JsDiagnosticLevel,
        source: impl Into<String>,
        message: impl Into<String>,
        stack: Option<String>,
        fields: BTreeMap<String, String>,
    ) {
        let elapsed_micros = self.started.elapsed().as_micros().min(u64::MAX as u128) as u64;
        if let Ok(mut state) = self.state.lock() {
            state.next_sequence = state.next_sequence.saturating_add(1);
            let sequence = state.next_sequence;
            if state.events.len() == self.capacity {
                state.events.pop_front();
            }
            state.events.push_back(DiagnosticEvent {
                sequence,
                elapsed_micros,
                kind,
                level,
                source: source.into(),
                message: message.into(),
                stack,
                fields,
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nana_js_engine::{HostApiRegistry, HostValue};

    #[test]
    fn records_host_calls_without_arguments_or_results() {
        let session = DevtoolsSession::default();
        let mut api = HostApiRegistry::new();
        api.set_observer(Some(session.host_call_observer()));
        api.register("secret", |_| Ok(HostValue::String("not-recorded".into())));
        api.call("secret", &[HostValue::String("private".into())])
            .unwrap();

        let event = session.drain_events().pop().unwrap();
        assert_eq!(event.kind, DiagnosticKind::HostCall);
        assert_eq!(event.message, "secret");
        assert!(!format!("{event:?}").contains("private"));
        assert!(!format!("{event:?}").contains("not-recorded"));
    }

    #[test]
    fn bounded_queue_and_frame_statistics_are_behavioral() {
        let session = DevtoolsSession::new(2);
        session.record_render_error("one");
        session.record_render_error("two");
        session.record_render_error("three");
        session.record_frame(
            7,
            Duration::from_millis(16),
            Duration::from_millis(2),
            Duration::from_millis(3),
            Duration::from_millis(1),
        );
        let snapshot = session.snapshot();
        assert_eq!(snapshot.events.len(), 2);
        assert_eq!(snapshot.frames[&7].presented_frames, 1);
        assert_eq!(snapshot.frames[&7].slowest_frame_micros, 6_000);
    }
}
