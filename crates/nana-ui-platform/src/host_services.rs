//! Non-blocking capability requests emitted by Runtime and fulfilled by hosts.

#[cfg(feature = "clipboard")]
use crate::SharedClipboardHost;
use crate::{EndpointGeneration, InputSourceId, PointerId};
use std::collections::{HashMap, VecDeque};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum HostCapability {
    Ime,
    Clipboard,
    Cursor,
    DragAndDrop,
    Accessibility,
    NativeTextInput,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HostRequestContext {
    pub source: InputSourceId,
    pub generation: EndpointGeneration,
    pub document: u64,
    pub node: Option<u64>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ImeSurroundingText {
    /// UTF-8 storage owned by the request until the host returns.
    pub text: String,
    pub selection: (usize, usize),
    /// Focused caret/candidate anchor in application logical coordinates.
    /// Hosts map it through the presentation bridge before showing UI.
    pub cursor_area: Option<nana_ui_core::LogicalRect>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DragPayload {
    /// The request owns this text until the host returns.
    Text(String),
    /// Paths are passed by value and must be re-authorized by the host;
    /// receiving a path does not grant file access.
    Files(Vec<String>),
}

#[derive(Debug, Clone, PartialEq)]
pub enum HostServiceRequest {
    ImeEnable {
        context: HostRequestContext,
        surrounding: Option<ImeSurroundingText>,
    },
    ImeUpdate {
        context: HostRequestContext,
        surrounding: Option<ImeSurroundingText>,
    },
    ImeDisable {
        context: HostRequestContext,
    },
    ClipboardRead {
        context: HostRequestContext,
    },
    ClipboardWrite {
        context: HostRequestContext,
        text: String,
        /// Whether the Runtime should delete the selected text only after a
        /// successful host write. This keeps cut atomic across the async
        /// boundary; copy uses `false`.
        cut: bool,
    },
    Cursor {
        context: HostRequestContext,
        cursor: String,
    },
    DragAndDrop {
        context: HostRequestContext,
        pointer: PointerId,
        payload: Option<DragPayload>,
    },
    Accessibility {
        context: HostRequestContext,
        payload: Vec<u8>,
    },
    NativeTextInput {
        context: HostRequestContext,
        enabled: bool,
    },
}

impl HostServiceRequest {
    pub fn context(&self) -> HostRequestContext {
        match self {
            Self::ImeEnable { context, .. }
            | Self::ImeUpdate { context, .. }
            | Self::ImeDisable { context }
            | Self::ClipboardRead { context }
            | Self::ClipboardWrite { context, .. }
            | Self::Cursor { context, .. }
            | Self::DragAndDrop { context, .. }
            | Self::Accessibility { context, .. }
            | Self::NativeTextInput { context, .. } => *context,
        }
    }
    pub fn capability(&self) -> HostCapability {
        match self {
            Self::ImeEnable { .. } | Self::ImeUpdate { .. } | Self::ImeDisable { .. } => {
                HostCapability::Ime
            }
            Self::ClipboardRead { .. } | Self::ClipboardWrite { .. } => HostCapability::Clipboard,
            Self::Cursor { .. } => HostCapability::Cursor,
            Self::DragAndDrop { .. } => HostCapability::DragAndDrop,
            Self::Accessibility { .. } => HostCapability::Accessibility,
            Self::NativeTextInput { .. } => HostCapability::NativeTextInput,
        }
    }

    /// Owned variable-payload capacity accounted against the bounded request
    /// queue. This is capacity telemetry, not allocator telemetry.
    pub fn payload_bytes(&self) -> usize {
        match self {
            Self::ImeEnable { surrounding, .. } | Self::ImeUpdate { surrounding, .. } => {
                surrounding.as_ref().map_or(0, |text| text.text.capacity())
            }
            Self::ClipboardWrite { text, .. } | Self::Cursor { cursor: text, .. } => {
                text.capacity()
            }
            Self::DragAndDrop { payload, .. } => {
                payload.as_ref().map_or(0, |payload| match payload {
                    DragPayload::Text(text) => text.capacity(),
                    DragPayload::Files(paths) => paths.iter().fold(
                        paths
                            .capacity()
                            .saturating_mul(std::mem::size_of::<String>()),
                        |bytes, path| bytes.saturating_add(path.capacity()),
                    ),
                })
            }
            Self::Accessibility { payload, .. } => payload.capacity(),
            Self::ClipboardRead { .. } | Self::ImeDisable { .. } | Self::NativeTextInput { .. } => {
                0
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HostServiceOutcome {
    Success,
    /// Text returned by a clipboard read. `None` means the host had no text
    /// available; it is distinct from a stale, unsupported, or denied request.
    ClipboardText(Option<String>),
    Unsupported,
    Denied,
    StaleGeneration,
    Failed(String),
}

/// A completed request paired with its original context and capability kind.
/// Keeping the request allows an async host to apply the result to the right
/// endpoint/document/node instead of correlating bare outcomes by position.
#[derive(Debug, Clone, PartialEq)]
pub struct HostServiceResponse {
    pub request: HostServiceRequest,
    pub outcome: HostServiceOutcome,
}

#[cfg(feature = "clipboard")]
/// Thin HostServices adapter over the existing clipboard contract. It owns no
/// clipboard state of its own and therefore cannot diverge from Runtime's
/// `ClipboardHost` path.
#[derive(Clone)]
pub struct ClipboardHostServices {
    clipboard: SharedClipboardHost,
}

#[cfg(feature = "clipboard")]
impl std::fmt::Debug for ClipboardHostServices {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ClipboardHostServices")
            .finish_non_exhaustive()
    }
}

#[cfg(feature = "clipboard")]
impl ClipboardHostServices {
    pub fn new(clipboard: SharedClipboardHost) -> Self {
        Self { clipboard }
    }
}

#[cfg(feature = "clipboard")]
impl HostServices for ClipboardHostServices {
    fn supports(&self, capability: HostCapability) -> bool {
        matches!(capability, HostCapability::Clipboard)
            && self
                .clipboard
                .lock()
                .map(|clipboard| clipboard.is_available())
                .unwrap_or(false)
    }

    fn request(&mut self, request: HostServiceRequest) -> HostServiceOutcome {
        match request {
            HostServiceRequest::ClipboardRead { .. } => {
                let Ok(mut clipboard) = self.clipboard.lock() else {
                    return HostServiceOutcome::Denied;
                };
                if !clipboard.is_available() {
                    return HostServiceOutcome::Unsupported;
                }
                HostServiceOutcome::ClipboardText(clipboard.read_text())
            }
            HostServiceRequest::ClipboardWrite { text, .. } => {
                let Ok(mut clipboard) = self.clipboard.lock() else {
                    return HostServiceOutcome::Denied;
                };
                if !clipboard.is_available() {
                    return HostServiceOutcome::Unsupported;
                }
                if clipboard.write_text(&text) {
                    HostServiceOutcome::Success
                } else {
                    HostServiceOutcome::Denied
                }
            }
            _ => HostServiceOutcome::Unsupported,
        }
    }
}

pub trait HostServices: Send {
    fn supports(&self, capability: HostCapability) -> bool;
    fn request(&mut self, request: HostServiceRequest) -> HostServiceOutcome;
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct HostServiceQueueCounters {
    pub requests_enqueued: u64,
    pub requests_rejected_capacity: u64,
    pub requests_drained: u64,
}

#[derive(Debug, PartialEq)]
pub struct HostServiceQueueFull {
    pub request: HostServiceRequest,
}

/// Bounded, event-driven request queue owned by the Runtime/host boundary.
///
/// The queue never silently drops a request. A full queue returns ownership to
/// the producer so the caller can deny, retry, or surface backpressure. It has
/// no worker and performs no polling; the host drains it at its normal frame
/// or event-loop boundary.
#[derive(Debug)]
pub struct HostServiceQueue {
    queue: VecDeque<HostServiceRequest>,
    capacity: usize,
    max_payload_bytes: usize,
    payload_bytes: usize,
    counters: HostServiceQueueCounters,
}

impl HostServiceQueue {
    pub fn new(capacity: usize) -> Self {
        Self::with_limits(capacity, 1024 * 1024)
    }

    pub fn with_limits(capacity: usize, max_payload_bytes: usize) -> Self {
        Self {
            queue: VecDeque::with_capacity(capacity),
            capacity,
            max_payload_bytes,
            payload_bytes: 0,
            counters: HostServiceQueueCounters::default(),
        }
    }

    pub fn capacity(&self) -> usize {
        self.capacity
    }

    pub fn len(&self) -> usize {
        self.queue.len()
    }

    pub fn max_payload_bytes(&self) -> usize {
        self.max_payload_bytes
    }

    pub fn queued_payload_bytes(&self) -> usize {
        self.payload_bytes
    }

    pub fn is_empty(&self) -> bool {
        self.queue.is_empty()
    }

    /// Whether one more zero-payload request can be accepted. Producers use
    /// this before applying an input mutation whose lifecycle intent must be
    /// queued atomically with the event.
    pub fn has_capacity(&self) -> bool {
        self.queue.len() < self.capacity
    }

    /// Whether a producer can reserve several lifecycle intents atomically.
    /// Focus replacement may enqueue disable plus enable after one input event.
    pub fn has_capacity_for(&self, count: usize) -> bool {
        count <= self.capacity.saturating_sub(self.queue.len())
    }

    pub fn can_accept_payload(&self, bytes: usize) -> bool {
        self.has_capacity() && bytes <= self.max_payload_bytes.saturating_sub(self.payload_bytes)
    }

    pub fn counters(&self) -> HostServiceQueueCounters {
        self.counters
    }

    #[allow(clippy::result_large_err)]
    pub fn push(&mut self, request: HostServiceRequest) -> Result<(), HostServiceQueueFull> {
        let bytes = request.payload_bytes();
        if self.queue.len() >= self.capacity
            || bytes > self.max_payload_bytes.saturating_sub(self.payload_bytes)
        {
            self.counters.requests_rejected_capacity += 1;
            nana_diagnostics::metric!(nana_diagnostics::framework::runtime::HOST_REQUESTS_REJECTED);
            return Err(HostServiceQueueFull { request });
        }
        self.payload_bytes += bytes;
        self.queue.push_back(request);
        self.counters.requests_enqueued += 1;
        nana_diagnostics::metric!(nana_diagnostics::framework::runtime::HOST_REQUESTS_ENQUEUED);
        Ok(())
    }

    pub fn pop(&mut self) -> Option<HostServiceRequest> {
        let request = self.queue.pop_front();
        if let Some(request) = &request {
            self.payload_bytes -= request.payload_bytes();
            self.counters.requests_drained += 1;
            nana_diagnostics::metric!(nana_diagnostics::framework::runtime::HOST_REQUESTS_DRAINED);
        }
        request
    }

    pub fn drain(&mut self, limit: usize) -> Vec<HostServiceRequest> {
        let count = limit.min(self.queue.len());
        let drained: Vec<_> = self.queue.drain(..count).collect();
        self.payload_bytes = self
            .payload_bytes
            .saturating_sub(drained.iter().map(HostServiceRequest::payload_bytes).sum());
        self.counters.requests_drained += drained.len() as u64;
        for _ in &drained {
            nana_diagnostics::metric!(nana_diagnostics::framework::runtime::HOST_REQUESTS_DRAINED);
        }
        drained
    }
}

/// Runtime-side generation gate. A host implementation never receives a
/// request after its source/document binding has been replaced.
#[derive(Debug)]
pub struct HostServiceBroker<H> {
    host: H,
    bindings: HashMap<InputSourceId, (EndpointGeneration, Option<u64>)>,
}

impl<H> HostServiceBroker<H> {
    pub fn new(host: H) -> Self {
        Self {
            host,
            bindings: HashMap::new(),
        }
    }

    pub fn bind(&mut self, source: InputSourceId, generation: EndpointGeneration) {
        self.bindings.insert(source, (generation, None));
    }

    /// Bind an endpoint to a specific document. Requests for another
    /// document are stale even when their endpoint generation is current.
    pub fn bind_document(
        &mut self,
        source: InputSourceId,
        generation: EndpointGeneration,
        document: u64,
    ) {
        self.bindings.insert(source, (generation, Some(document)));
    }

    pub fn unbind(&mut self, source: InputSourceId) {
        self.bindings.remove(&source);
    }

    pub fn host(&self) -> &H {
        &self.host
    }

    pub fn host_mut(&mut self) -> &mut H {
        &mut self.host
    }
}

impl<H: HostServices> HostServices for HostServiceBroker<H> {
    fn supports(&self, capability: HostCapability) -> bool {
        self.host.supports(capability)
    }

    fn request(&mut self, request: HostServiceRequest) -> HostServiceOutcome {
        let context = request.context();
        let Some((generation, document)) = self.bindings.get(&context.source) else {
            return HostServiceOutcome::StaleGeneration;
        };
        if generation != &context.generation
            || document.is_some_and(|bound| bound != context.document)
        {
            return HostServiceOutcome::StaleGeneration;
        }
        self.host.request(request)
    }
}

#[derive(Debug, Default)]
pub struct UnsupportedHostServices;

impl HostServices for UnsupportedHostServices {
    fn supports(&self, _capability: HostCapability) -> bool {
        false
    }
    fn request(&mut self, _request: HostServiceRequest) -> HostServiceOutcome {
        HostServiceOutcome::Unsupported
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(feature = "clipboard")]
    use crate::shared_clipboard;
    #[test]
    fn request_keeps_generation_and_explicit_denial() {
        let context = HostRequestContext {
            source: InputSourceId(1),
            generation: EndpointGeneration(2),
            document: 3,
            node: Some(4),
        };
        let request = HostServiceRequest::ClipboardRead { context };
        assert_eq!(request.context().generation, EndpointGeneration(2));
        assert_eq!(request.capability(), HostCapability::Clipboard);
        assert_eq!(
            UnsupportedHostServices.request(request),
            HostServiceOutcome::Unsupported
        );
    }

    #[test]
    fn broker_rejects_stale_requests_before_host_execution() {
        let context = HostRequestContext {
            source: InputSourceId(1),
            generation: EndpointGeneration(2),
            document: 3,
            node: None,
        };
        let mut broker = HostServiceBroker::new(UnsupportedHostServices);
        broker.bind(context.source, EndpointGeneration(1));
        assert_eq!(
            broker.request(HostServiceRequest::Cursor {
                context,
                cursor: "default".into(),
            }),
            HostServiceOutcome::StaleGeneration
        );

        broker.bind_document(context.source, context.generation, context.document);
        assert_eq!(
            broker.request(HostServiceRequest::Cursor {
                context: HostRequestContext {
                    document: context.document + 1,
                    ..context
                },
                cursor: "default".into(),
            }),
            HostServiceOutcome::StaleGeneration
        );
    }

    #[cfg(feature = "clipboard")]
    #[test]
    fn clipboard_host_adapter_round_trips_typed_read_results() {
        let clipboard = shared_clipboard(crate::MemoryClipboard::new());
        let context = HostRequestContext {
            source: InputSourceId(1),
            generation: EndpointGeneration(1),
            document: 1,
            node: None,
        };
        let mut host = ClipboardHostServices::new(clipboard);
        assert_eq!(
            host.request(HostServiceRequest::ClipboardWrite {
                context,
                text: "hello".into(),
                cut: false,
            }),
            HostServiceOutcome::Success
        );
        assert_eq!(
            host.request(HostServiceRequest::ClipboardRead { context }),
            HostServiceOutcome::ClipboardText(Some("hello".into()))
        );
    }

    #[cfg(feature = "clipboard")]
    #[test]
    fn unsupported_clipboard_reports_capability_denial() {
        let clipboard = shared_clipboard(crate::UnsupportedClipboard);
        let mut host = ClipboardHostServices::new(clipboard);
        assert!(!host.supports(HostCapability::Clipboard));
        let context = HostRequestContext {
            source: InputSourceId(1),
            generation: EndpointGeneration(1),
            document: 1,
            node: None,
        };
        assert_eq!(
            host.request(HostServiceRequest::ClipboardRead { context }),
            HostServiceOutcome::Unsupported
        );
    }

    #[test]
    fn request_queue_returns_ownership_on_capacity_and_drains_in_order() {
        let context = HostRequestContext {
            source: InputSourceId(1),
            generation: EndpointGeneration(1),
            document: 1,
            node: None,
        };
        let mut queue = HostServiceQueue::new(1);
        queue
            .push(HostServiceRequest::ClipboardRead { context })
            .expect("first request fits");
        let rejected = queue
            .push(HostServiceRequest::Cursor {
                context,
                cursor: "text".into(),
            })
            .expect_err("second request must preserve ownership");
        assert!(matches!(
            rejected.request,
            HostServiceRequest::Cursor { .. }
        ));
        assert!(matches!(
            queue.pop(),
            Some(HostServiceRequest::ClipboardRead { .. })
        ));
        assert!(queue.is_empty());
        assert_eq!(queue.counters().requests_rejected_capacity, 1);
        assert_eq!(queue.counters().requests_drained, 1);
    }

    #[test]
    fn request_queue_enforces_payload_budget_and_reclaims_it_on_drain() {
        let context = HostRequestContext {
            source: InputSourceId(1),
            generation: EndpointGeneration(1),
            document: 1,
            node: None,
        };
        let mut queue = HostServiceQueue::with_limits(4, 4);
        let rejected = queue
            .push(HostServiceRequest::ClipboardWrite {
                context,
                text: "large".into(),
                cut: false,
            })
            .expect_err("payload budget must reject oversized ownership");
        assert!(matches!(
            rejected.request,
            HostServiceRequest::ClipboardWrite { .. }
        ));
        assert_eq!(queue.queued_payload_bytes(), 0);
        queue
            .push(HostServiceRequest::ClipboardWrite {
                context,
                text: "four".into(),
                cut: false,
            })
            .expect("payload fits");
        assert_eq!(queue.queued_payload_bytes(), 4);
        let _ = queue.pop();
        assert_eq!(queue.queued_payload_bytes(), 0);
    }

    #[test]
    fn drag_files_payload_counts_vec_backing_storage_even_when_empty() {
        let context = HostRequestContext {
            source: InputSourceId(1),
            generation: EndpointGeneration(1),
            document: 1,
            node: None,
        };
        let paths: Vec<String> = Vec::with_capacity(4);
        let backing_bytes = paths
            .capacity()
            .saturating_mul(std::mem::size_of::<String>());
        let request = HostServiceRequest::DragAndDrop {
            context,
            pointer: PointerId(1),
            payload: Some(DragPayload::Files(paths)),
        };

        assert!(backing_bytes > 0);
        assert_eq!(request.payload_bytes(), backing_bytes);
    }
}
