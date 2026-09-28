//! Stable platform-input routing for Nana-native Runtime components.

use nana_ui_core::TableNavigation;
use nana_ui_platform::{
    CanonicalInputEvent, CompositionInput, HostRequestContext, HostServiceOutcome,
    HostServiceQueue, HostServiceRequest, HostServices, ImeEvent, InputDisposition, InputEvent,
    InputPayload, PointerPhase, SharedClipboardHost, default_shared_clipboard,
};
use nana_ui_runtime::{
    AppContext, DocumentId, FrameworkError, RangeAdjustment, RovingFocusIntent, ScrollOffset,
    StableNodeId, TextCaretIntent, TextDeleteKind, TextLineDirection, TextShaper, XYPadAdjustment,
};
#[cfg(feature = "graph-canvas")]
use nana_ui_runtime::{GraphCanvasAdjustment, GraphPointerButton, GraphScrollDelta};
use nana_ui_runtime::{OverlayKey, OverlayPointerDecision, OverlayPointerPhase};
use std::collections::HashMap;
use std::sync::OnceLock;
use std::time::{Duration, Instant};

macro_rules! optional_input {
    ($feature:literal, $call:expr, $absent:expr) => {{
        #[cfg(feature = $feature)]
        {
            $call
        }
        #[cfg(not(feature = $feature))]
        {
            $absent
        }
    }};
}

const DEFAULT_LINE_SCROLL_EXTENT: f32 = 60.0;

/// Pasteboard shared by every adapter that does not carry its own.
///
/// The OS pasteboard is one system resource, and adapters are built per event,
/// so the backend is opened once per process instead of once per keystroke.
fn process_clipboard() -> &'static SharedClipboardHost {
    static CLIPBOARD: OnceLock<SharedClipboardHost> = OnceLock::new();
    CLIPBOARD.get_or_init(default_shared_clipboard)
}

/// Converts renderer-neutral platform input into typed Runtime component
/// actions. It owns no input or component state.
#[derive(Clone)]
pub struct RuntimeInputAdapter {
    pub line_scroll_extent: f32,
    pub table_page_rows: usize,
    clipboard: Option<SharedClipboardHost>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InputRouterError {
    UnknownSource,
    StaleGeneration,
    Disconnected,
    OutOfOrder,
    TimestampRegression,
    /// No input state was changed; drain host requests and retry this event.
    HostServiceBackpressure,
    Dispatch(String),
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct InputRouterCounters {
    pub events_routed: u64,
    pub hit_tests: u64,
    /// Successful canonical dispatches. This is not a fabricated node-path
    /// length; a future propagation API can add a real routed-node count.
    pub routed_dispatches: u64,
    pub focus_changes: u64,
    pub pointer_capture_changes: u64,
    pub hover_path_changes: u64,
    pub routing_cache_hits: u64,
    pub routing_cache_misses: u64,
    pub events_rejected_unknown_source: u64,
    pub events_rejected_stale_generation: u64,
    pub events_rejected_disconnected: u64,
    pub events_rejected_order: u64,
    pub pointer_identity_cache_hits: u64,
    pub pointer_identity_cache_misses: u64,
    pub host_requests_stale_dropped: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CanonicalInputKind {
    Pointer,
    Wheel,
    Key,
    Text,
    Composition,
    PointerEnter,
    PointerLeave,
    Focus,
    DeviceConnected,
    DeviceDisconnected,
    SourceConnected,
    SourceDisconnected,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InputRouteSnapshot {
    pub source: nana_ui_platform::InputSourceId,
    pub device: nana_ui_platform::DeviceId,
    pub pointer: Option<nana_ui_platform::PointerId>,
    pub kind: CanonicalInputKind,
    pub document: DocumentId,
    pub hover_owner: Option<StableNodeId>,
    pub focus_owner: Option<StableNodeId>,
    pub capture_owner: Option<StableNodeId>,
    pub route_latency_ns: u64,
}

/// The non-blocking result of routing one canonical event.
///
/// `invalidated_work` is derived from UiWorld's monotonic pending-work
/// revision, so it reports work scheduled by this dispatch without consuming
/// `SystemWork`. Host requests remain queued for the host boundary and are
/// counted here in enqueue order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InputRouteOutcome {
    pub handled: bool,
    pub prevent_default: bool,
    pub invalidated_work: bool,
    pub host_requests_enqueued: usize,
}

fn canonical_input_kind(payload: &InputPayload) -> CanonicalInputKind {
    match payload {
        InputPayload::Pointer(_) => CanonicalInputKind::Pointer,
        InputPayload::Wheel(_) => CanonicalInputKind::Wheel,
        InputPayload::Key(_) => CanonicalInputKind::Key,
        InputPayload::TextInput(_) => CanonicalInputKind::Text,
        InputPayload::Composition(_) => CanonicalInputKind::Composition,
        InputPayload::PointerEnter { .. } => CanonicalInputKind::PointerEnter,
        InputPayload::PointerLeave { .. } => CanonicalInputKind::PointerLeave,
        InputPayload::Focus { .. } => CanonicalInputKind::Focus,
        InputPayload::DeviceConnected => CanonicalInputKind::DeviceConnected,
        InputPayload::DeviceDisconnected => CanonicalInputKind::DeviceDisconnected,
        InputPayload::SourceConnected => CanonicalInputKind::SourceConnected,
        InputPayload::SourceDisconnected => CanonicalInputKind::SourceDisconnected,
    }
}

fn canonical_pointer(payload: &InputPayload) -> Option<nana_ui_platform::PointerId> {
    match payload {
        InputPayload::Pointer(pointer) => Some(pointer.pointer_id),
        InputPayload::Wheel(wheel) => Some(wheel.pointer_id),
        InputPayload::PointerEnter { pointer_id, .. }
        | InputPayload::PointerLeave { pointer_id } => Some(*pointer_id),
        _ => None,
    }
}

fn cursor_spec_name(cursor: nana_ui_core::CursorSpec) -> &'static str {
    match cursor {
        nana_ui_core::CursorSpec::Default => "default",
        nana_ui_core::CursorSpec::Pointer => "pointer",
        nana_ui_core::CursorSpec::Text => "text",
        nana_ui_core::CursorSpec::Move => "move",
        nana_ui_core::CursorSpec::Grab => "grab",
        nana_ui_core::CursorSpec::Grabbing => "grabbing",
        nana_ui_core::CursorSpec::NotAllowed => "not-allowed",
        nana_ui_core::CursorSpec::Crosshair => "crosshair",
        nana_ui_core::CursorSpec::Help => "help",
        nana_ui_core::CursorSpec::Wait => "wait",
        nana_ui_core::CursorSpec::Progress => "progress",
        nana_ui_core::CursorSpec::ZoomIn => "zoom-in",
        nana_ui_core::CursorSpec::ZoomOut => "zoom-out",
        nana_ui_core::CursorSpec::None => "none",
    }
}

impl std::fmt::Display for InputRouterError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::UnknownSource => f.write_str("input source is not attached"),
            Self::StaleGeneration => f.write_str("input endpoint generation is stale"),
            Self::Disconnected => f.write_str("input source or device is disconnected"),
            Self::OutOfOrder => f.write_str("input sequence is out of order"),
            Self::TimestampRegression => f.write_str("input timestamp regressed"),
            Self::HostServiceBackpressure => f.write_str("host service request queue is full"),
            Self::Dispatch(error) => f.write_str(error),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct InputBinding {
    generation: nana_ui_platform::EndpointGeneration,
    document: DocumentId,
}

#[derive(Debug, Clone, Copy)]
struct PointerIdentity {
    local: u64,
    pointer_type: nana_ui_platform::PointerType,
    is_primary: bool,
}

/// Host-independent endpoint lifecycle and canonical event router.
///
/// The router owns source binding and canonical pointer identity. Hit testing,
/// focus, capture, and component behavior remain in `AppContext`/`UiWorld`.
pub struct InputRouter {
    bindings: HashMap<nana_ui_platform::InputSourceId, InputBinding>,
    pointers: HashMap<
        (
            nana_ui_platform::InputSourceId,
            nana_ui_platform::DeviceId,
            nana_ui_platform::PointerId,
        ),
        PointerIdentity,
    >,
    disconnected_sources: std::collections::HashSet<nana_ui_platform::InputSourceId>,
    disconnected_devices:
        std::collections::HashSet<(nana_ui_platform::InputSourceId, nana_ui_platform::DeviceId)>,
    focused_sources: std::collections::HashMap<
        DocumentId,
        std::collections::HashSet<nana_ui_platform::InputSourceId>,
    >,
    last_order: HashMap<
        nana_ui_platform::InputSourceId,
        (
            nana_ui_platform::InputSequence,
            nana_ui_platform::InputTimestamp,
        ),
    >,
    next_pointer: u64,
    adapter: RuntimeInputAdapter,
    host_requests: HostServiceQueue,
    ime_focus: HashMap<DocumentId, StableNodeId>,
    cursor_state: HashMap<
        (
            nana_ui_platform::InputSourceId,
            nana_ui_platform::DeviceId,
            DocumentId,
        ),
        String,
    >,
    counters: InputRouterCounters,
    last_route: Option<InputRouteSnapshot>,
}

impl Default for InputRouter {
    fn default() -> Self {
        Self {
            bindings: HashMap::new(),
            pointers: HashMap::new(),
            disconnected_sources: std::collections::HashSet::new(),
            disconnected_devices: std::collections::HashSet::new(),
            focused_sources: std::collections::HashMap::new(),
            last_order: HashMap::new(),
            next_pointer: 1,
            adapter: RuntimeInputAdapter::default(),
            host_requests: HostServiceQueue::new(128),
            ime_focus: HashMap::new(),
            cursor_state: HashMap::new(),
            counters: InputRouterCounters::default(),
            last_route: None,
        }
    }
}

impl InputRouter {
    pub fn ensure_attached(
        &mut self,
        source: nana_ui_platform::InputSourceId,
        generation: nana_ui_platform::EndpointGeneration,
        document: DocumentId,
    ) {
        // This is an idempotent binding helper for hosts that have already
        // completed endpoint teardown. Lifecycle callers must use
        // `detach_with_context` before changing a generation so active
        // capture receives PointerCancel while its document is alive.
        if self.binding(source) != Some((generation, document)) {
            self.attach(source, generation, document);
        }
    }

    /// Attach an endpoint only at the same or a newer generation. Returns
    /// `false` when a stale generation or same-generation document rebind is
    /// attempted; callers must detach the old document before reusing it.
    pub fn attach(
        &mut self,
        source: nana_ui_platform::InputSourceId,
        generation: nana_ui_platform::EndpointGeneration,
        document: DocumentId,
    ) -> bool {
        let previous_binding = self.bindings.get(&source).copied();
        if let Some(previous) = previous_binding {
            if generation < previous.generation
                || (generation == previous.generation && document != previous.document)
            {
                return false;
            }
            if generation == previous.generation && document == previous.document {
                return true;
            }
        }
        if self
            .last_route
            .is_some_and(|snapshot| snapshot.source == source)
        {
            self.last_route = None;
        }
        self.bindings.insert(
            source,
            InputBinding {
                generation,
                document,
            },
        );
        self.pointers.retain(|(bound, _, _), _| *bound != source);
        self.disconnected_sources.remove(&source);
        self.disconnected_devices
            .retain(|(bound, _)| *bound != source);
        self.cursor_state
            .retain(|(bound, _, _), _| *bound != source);
        self.focused_sources.values_mut().for_each(|owners| {
            owners.remove(&source);
        });
        self.focused_sources.retain(|_, owners| !owners.is_empty());
        self.last_order.remove(&source);
        // A new endpoint generation cannot inherit the previous native IME
        // enablement. Clear the document cache so the next focused editable
        // event emits a fresh ImeEnable intent for the new endpoint.
        if previous_binding.is_some_and(|previous| previous.generation != generation)
            || previous_binding.is_some_and(|previous| previous.document != document)
        {
            self.ime_focus.remove(&document);
        }
        if let Some(previous) = previous_binding
            && previous.document != document
            && !self
                .bindings
                .values()
                .any(|binding| binding.document == previous.document)
        {
            self.ime_focus.remove(&previous.document);
        }
        true
    }

    pub fn detach(&mut self, source: nana_ui_platform::InputSourceId) -> Option<DocumentId> {
        if self
            .last_route
            .is_some_and(|snapshot| snapshot.source == source)
        {
            self.last_route = None;
        }
        self.pointers.retain(|(bound, _, _), _| *bound != source);
        self.disconnected_sources.remove(&source);
        self.disconnected_devices
            .retain(|(bound, _)| *bound != source);
        self.cursor_state
            .retain(|(bound, _, _), _| *bound != source);
        self.focused_sources.values_mut().for_each(|owners| {
            owners.remove(&source);
        });
        self.focused_sources.retain(|_, owners| !owners.is_empty());
        self.last_order.remove(&source);
        let document = self
            .bindings
            .remove(&source)
            .map(|binding| binding.document);
        if let Some(document) = document
            && !self
                .bindings
                .values()
                .any(|binding| binding.document == document)
        {
            self.ime_focus.remove(&document);
            self.cursor_state
                .retain(|(_, _, bound_document), _| *bound_document != document);
        }
        document
    }

    /// Detach an endpoint while its document is still alive. Lifecycle
    /// callers should use this variant so active captures receive cancel and
    /// hover state receives leave before the binding is removed.
    pub fn detach_with_context(
        &mut self,
        context: &mut AppContext,
        source: nana_ui_platform::InputSourceId,
        now: Duration,
    ) -> Result<Option<DocumentId>, InputRouterError> {
        let Some(binding) = self.bindings.get(&source).copied() else {
            return Ok(None);
        };
        self.cancel_captured_pointers(context, binding.document, source, None, now)
            .map_err(|error| InputRouterError::Dispatch(error.to_string()))?;
        Ok(self.detach(source))
    }

    pub fn binding(
        &self,
        source: nana_ui_platform::InputSourceId,
    ) -> Option<(nana_ui_platform::EndpointGeneration, DocumentId)> {
        self.bindings
            .get(&source)
            .map(|binding| (binding.generation, binding.document))
    }

    pub fn counters(&self) -> InputRouterCounters {
        self.counters
    }

    pub fn last_route_snapshot(&self) -> Option<InputRouteSnapshot> {
        self.last_route
    }

    /// Take capability requests produced while routing canonical input. The
    /// host drains these at its normal event-loop/frame boundary; routing
    /// itself never waits for a host service response.
    pub fn take_host_service_requests(
        &mut self,
        context: &AppContext,
        limit: usize,
    ) -> Vec<HostServiceRequest> {
        let mut current = Vec::new();
        for request in self.host_requests.drain(limit) {
            let request_context = request.context();
            let valid = self
                .bindings
                .get(&request_context.source)
                .is_some_and(|binding| {
                    binding.generation == request_context.generation
                        && binding.document.get() == request_context.document
                });
            let valid = valid
                && nana_ui_runtime::DocumentId::new(request_context.document)
                    .is_some_and(|document| context.has_document(document));
            let valid = valid
                && match &request {
                    HostServiceRequest::ImeEnable {
                        context: request, ..
                    }
                    | HostServiceRequest::ImeUpdate {
                        context: request, ..
                    } => request.node.is_some_and(|node| {
                        context
                            .world()
                            .focused(self.bindings[&request.source].document)
                            .is_some_and(|focused| focused.get() == node)
                    }),
                    HostServiceRequest::Cursor {
                        context: request, ..
                    } => request.node.is_none_or(|node| {
                        StableNodeId::new(node).is_some_and(|node| context.world().is_mounted(node))
                    }),
                    HostServiceRequest::NativeTextInput {
                        context: request,
                        enabled,
                    } => {
                        !*enabled
                            || request.node.is_some_and(|node| {
                                context
                                    .world()
                                    .focused(self.bindings[&request.source].document)
                                    .is_some_and(|focused| focused.get() == node)
                            })
                    }
                    HostServiceRequest::ImeDisable { .. } => true,
                    _ => true,
                };
            if valid {
                current.push(request);
            } else {
                self.counters.host_requests_stale_dropped += 1;
                nana_diagnostics::metric!(
                    nana_diagnostics::framework::runtime::HOST_REQUESTS_STALE
                );
            }
        }
        current
    }

    /// Fulfil queued capability requests at the host boundary. This is kept
    /// separate from [`Self::route`] so a platform service can never block
    /// pointer/key routing. Requests are generation/document checked before
    /// they reach the capability implementation; the returned outcomes stay
    /// ordered with the accepted requests.
    pub fn service_host_requests(
        &mut self,
        context: &AppContext,
        services: &mut dyn HostServices,
        limit: usize,
    ) -> Vec<HostServiceOutcome> {
        self.service_host_requests_with_results(context, services, limit)
            .into_iter()
            .map(|response| response.outcome)
            .collect()
    }

    /// Fulfil requests while retaining the originating request for async
    /// result application. The legacy method above remains an outcome-only
    /// convenience for hosts that do not need correlation.
    pub fn service_host_requests_with_results(
        &mut self,
        context: &AppContext,
        services: &mut dyn HostServices,
        limit: usize,
    ) -> Vec<nana_ui_platform::HostServiceResponse> {
        self.take_host_service_requests(context, limit)
            .into_iter()
            .map(|request| {
                let capability = request.capability();
                let outcome = if services.supports(capability) {
                    services.request(request.clone())
                } else {
                    HostServiceOutcome::Unsupported
                };
                nana_ui_platform::HostServiceResponse { request, outcome }
            })
            .collect()
    }

    /// Apply a host result after its asynchronous boundary. The request
    /// context is checked again because the endpoint or focused document may
    /// have been revoked while the host was working.
    pub fn apply_host_service_response(
        &mut self,
        context: &mut AppContext,
        response: nana_ui_platform::HostServiceResponse,
    ) -> Result<bool, InputRouterError> {
        let request_context = response.request.context();
        let Some(binding) = self.bindings.get(&request_context.source).copied() else {
            self.counters.host_requests_stale_dropped += 1;
            return Ok(false);
        };
        let Some(document) = DocumentId::new(request_context.document) else {
            self.counters.host_requests_stale_dropped += 1;
            return Ok(false);
        };
        let node_valid = match &response.request {
            HostServiceRequest::Cursor { .. } => request_context.node.is_none_or(|node| {
                StableNodeId::new(node).is_some_and(|node| context.world().is_mounted(node))
            }),
            HostServiceRequest::ImeEnable { .. }
            | HostServiceRequest::ImeUpdate { .. }
            | HostServiceRequest::ClipboardRead { .. }
            | HostServiceRequest::ClipboardWrite { cut: true, .. } => {
                request_context.node.is_some_and(|node| {
                    context
                        .world()
                        .focused(document)
                        .is_some_and(|focused| focused.get() == node)
                })
            }
            HostServiceRequest::ClipboardWrite { cut: false, .. } => {
                request_context.node.is_none_or(|node| {
                    StableNodeId::new(node).is_some_and(|node| context.world().is_mounted(node))
                })
            }
            HostServiceRequest::NativeTextInput { enabled: true, .. } => {
                request_context.node.is_some_and(|node| {
                    context
                        .world()
                        .focused(document)
                        .is_some_and(|focused| focused.get() == node)
                })
            }
            HostServiceRequest::NativeTextInput { enabled: false, .. } => true,
            _ => true,
        };
        if binding.generation != request_context.generation
            || binding.document != document
            || !context.has_document(document)
            || !node_valid
        {
            self.counters.host_requests_stale_dropped += 1;
            return Ok(false);
        }
        match (response.request, response.outcome) {
            (
                HostServiceRequest::ClipboardRead { .. },
                HostServiceOutcome::ClipboardText(Some(text)),
            ) => context
                .paste_focused_text(document, &text)
                .map_err(|error| InputRouterError::Dispatch(error.to_string())),
            (HostServiceRequest::ClipboardWrite { cut: true, .. }, HostServiceOutcome::Success) => {
                context
                    .cut_focused_text(document)
                    .map(|_| true)
                    .map_err(|error| InputRouterError::Dispatch(error.to_string()))
            }
            _ => Ok(false),
        }
    }

    pub fn route(
        &mut self,
        context: &mut AppContext,
        event: &nana_ui_platform::CanonicalInputEvent,
        now: Duration,
        text_shaper: Option<&mut dyn TextShaper>,
    ) -> Result<InputDisposition, InputRouterError> {
        let route_started = Instant::now();
        let kind = canonical_input_kind(&event.payload);
        let pointer = canonical_pointer(&event.payload);
        let Some(binding) = self.bindings.get(&event.metadata.source).copied() else {
            self.counters.events_rejected_unknown_source += 1;
            nana_diagnostics::metric!(nana_diagnostics::framework::runtime::INPUT_ROUTE_REJECTED);
            return Err(InputRouterError::UnknownSource);
        };
        if binding.generation != event.metadata.generation {
            self.counters.events_rejected_stale_generation += 1;
            nana_diagnostics::metric!(nana_diagnostics::framework::runtime::INPUT_ROUTE_REJECTED);
            return Err(InputRouterError::StaleGeneration);
        }
        let source_connected = matches!(
            &event.payload,
            nana_ui_platform::InputPayload::SourceConnected
        );
        let device_connected = matches!(
            &event.payload,
            nana_ui_platform::InputPayload::DeviceConnected
        );
        if (self.disconnected_sources.contains(&event.metadata.source) && !source_connected)
            || (self
                .disconnected_devices
                .contains(&(event.metadata.source, event.metadata.device))
                && !device_connected
                && !source_connected
                && !matches!(event.payload, InputPayload::SourceDisconnected))
        {
            self.counters.events_rejected_disconnected += 1;
            nana_diagnostics::metric!(nana_diagnostics::framework::runtime::INPUT_ROUTE_REJECTED);
            return Err(InputRouterError::Disconnected);
        }
        if let Some((sequence, timestamp)) = self.last_order.get(&event.metadata.source)
            && (event.metadata.sequence <= *sequence || event.metadata.timestamp < *timestamp)
        {
            self.counters.events_rejected_order += 1;
            nana_diagnostics::metric!(nana_diagnostics::framework::runtime::INPUT_ROUTE_REJECTED);
            return Err(if event.metadata.sequence <= *sequence {
                InputRouterError::OutOfOrder
            } else {
                InputRouterError::TimestampRegression
            });
        }
        let composition_payload_bytes = matches!(
            event.payload,
            nana_ui_platform::InputPayload::Composition(_)
        )
        .then(|| {
            context
                .focused_text_input(binding.document)
                .map_or(0, |(_, view)| view.value.len())
        });
        // A focus transition can be caused by a pointer/key dispatch and may
        // enqueue ImeDisable + ImeEnable after the Runtime mutation. Reserve
        // both slots before dispatch so a full host queue cannot report a
        // failed route after the edit/focus state has already changed.
        let lifecycle_slots = match &event.payload {
            nana_ui_platform::InputPayload::Pointer(pointer)
                if pointer.phase == PointerPhase::Down =>
            {
                4
            }
            nana_ui_platform::InputPayload::Pointer(_) => 4,
            nana_ui_platform::InputPayload::Key(key)
                if key.state == nana_ui_platform::KeyState::Pressed =>
            {
                4
            }
            nana_ui_platform::InputPayload::Focus { focused: true } => 3,
            nana_ui_platform::InputPayload::Focus { focused: false } => 2,
            nana_ui_platform::InputPayload::Composition(_) => 1,
            _ => 0,
        };
        if (lifecycle_slots != 0 && !self.host_requests.has_capacity_for(lifecycle_slots))
            || composition_payload_bytes
                .is_some_and(|bytes| !self.host_requests.can_accept_payload(bytes))
        {
            return Err(InputRouterError::HostServiceBackpressure);
        }
        let pointer_before = pointer.map(|pointer| {
            self.pointers
                .get(&(event.metadata.source, event.metadata.device, pointer))
                .map(|identity| identity.local)
                .unwrap_or(pointer.0)
        });
        let focus_before = context.world().focused(binding.document);
        let capture_before = pointer_before
            .and_then(|pointer| context.world().pointer_capture(binding.document, pointer));
        let hover_before = pointer_before
            .and_then(|pointer| context.world().pointer_hover(binding.document, pointer));
        if pointer.is_some() {
            if capture_before.is_some() {
                self.counters.routing_cache_hits += 1;
                nana_diagnostics::metric!(
                    nana_diagnostics::framework::runtime::INPUT_ROUTING_CACHE_HITS
                );
            } else {
                self.counters.routing_cache_misses += 1;
                self.counters.hit_tests += 1;
                nana_diagnostics::metric!(
                    nana_diagnostics::framework::runtime::INPUT_ROUTING_CACHE_MISSES
                );
                nana_diagnostics::metric!(nana_diagnostics::framework::runtime::INPUT_HIT_TESTS);
            }
        }
        self.counters.events_routed += 1;
        let lifecycle = match &event.payload {
            nana_ui_platform::InputPayload::DeviceDisconnected => Some(true),
            nana_ui_platform::InputPayload::SourceDisconnected => Some(false),
            _ => None,
        };
        if let Some(device_only) = lifecycle {
            self.cancel_captured_pointers(
                context,
                binding.document,
                event.metadata.source,
                device_only.then_some(event.metadata.device),
                now,
            )
            .map_err(|error| InputRouterError::Dispatch(error.to_string()))?;
            if device_only {
                self.disconnected_devices
                    .insert((event.metadata.source, event.metadata.device));
            } else {
                self.disconnected_sources.insert(event.metadata.source);
                let no_focus_owners = self
                    .focused_sources
                    .get_mut(&binding.document)
                    .map(|owners| {
                        owners.remove(&event.metadata.source);
                        owners.is_empty()
                    })
                    .unwrap_or(false);
                if no_focus_owners {
                    self.focused_sources.remove(&binding.document);
                    context
                        .clear_focus(binding.document)
                        .map_err(|error| InputRouterError::Dispatch(error.to_string()))?;
                }
            }
        } else if device_connected {
            self.disconnected_devices
                .remove(&(event.metadata.source, event.metadata.device));
        } else if source_connected {
            self.disconnected_sources.remove(&event.metadata.source);
        }
        if let nana_ui_platform::InputPayload::Focus { focused } = &event.payload {
            let no_focus_owners = {
                let owners = self.focused_sources.entry(binding.document).or_default();
                if *focused {
                    owners.insert(event.metadata.source);
                } else {
                    owners.remove(&event.metadata.source);
                }
                owners.is_empty()
            };
            if !*focused {
                if no_focus_owners {
                    self.focused_sources.remove(&binding.document);
                }
                self.cancel_captured_pointers(
                    context,
                    binding.document,
                    event.metadata.source,
                    None,
                    now,
                )
                .map_err(|error| InputRouterError::Dispatch(error.to_string()))?;
                if no_focus_owners {
                    context
                        .clear_focus(binding.document)
                        .map_err(|error| InputRouterError::Dispatch(error.to_string()))?;
                }
            }
        }
        let terminal_pointer = match &event.payload {
            nana_ui_platform::InputPayload::Pointer(pointer)
                if pointer.pointer_type == nana_ui_platform::PointerType::Touch
                    && matches!(pointer.phase, PointerPhase::Up | PointerPhase::Cancel) =>
            {
                Some((
                    event.metadata.source,
                    event.metadata.device,
                    pointer.pointer_id,
                ))
            }
            _ => None,
        };
        let event = self.remap_pointer(event);
        let result = match self.route_clipboard_shortcut(context, &event, binding.document)? {
            Some(disposition) => Ok(disposition),
            None => self
                .adapter
                .dispatch_canonical(context, binding.document, &event, now, text_shaper)
                .map_err(|error| InputRouterError::Dispatch(error.to_string())),
        };
        if result.is_ok() {
            self.last_order.insert(
                event.metadata.source,
                (event.metadata.sequence, event.metadata.timestamp),
            );
            self.counters.routed_dispatches += 1;
            nana_diagnostics::metric!(
                nana_diagnostics::framework::runtime::INPUT_ROUTED_DISPATCHES
            );
            self.sync_ime_focus_request(context, &event, binding.document)?;
            self.enqueue_ime_request(context, &event, binding.document)?;
        }
        if focus_before != context.world().focused(binding.document) {
            self.counters.focus_changes += 1;
            nana_diagnostics::metric!(nana_diagnostics::framework::runtime::INPUT_FOCUS_CHANGES);
        }
        let capture_after = pointer_before
            .and_then(|pointer| context.world().pointer_capture(binding.document, pointer));
        if capture_before != capture_after {
            self.counters.pointer_capture_changes += 1;
            nana_diagnostics::metric!(nana_diagnostics::framework::runtime::INPUT_CAPTURE_CHANGES);
        }
        let hover_after = pointer_before
            .and_then(|pointer| context.world().pointer_hover(binding.document, pointer));
        if hover_before != hover_after {
            self.counters.hover_path_changes += 1;
            nana_diagnostics::metric!(nana_diagnostics::framework::runtime::INPUT_HOVER_CHANGES);
        }
        if result.is_ok() {
            self.sync_cursor_request(
                context,
                event.metadata.source,
                event.metadata.device,
                event.metadata.generation,
                binding.document,
                hover_after,
            )?;
        }
        let route_pointer = pointer.map(|pointer| {
            self.pointers
                .get(&(event.metadata.source, event.metadata.device, pointer))
                .copied()
                .map(|identity| nana_ui_platform::PointerId(identity.local))
                .unwrap_or(pointer)
        });
        let route_latency_ns = route_started.elapsed().as_nanos().min(u128::from(u64::MAX)) as u64;
        self.last_route = Some(InputRouteSnapshot {
            source: event.metadata.source,
            device: event.metadata.device,
            pointer: route_pointer,
            kind,
            document: binding.document,
            hover_owner: route_pointer
                .and_then(|pointer| context.world().pointer_hover(binding.document, pointer.0)),
            focus_owner: context.world().focused(binding.document),
            capture_owner: route_pointer
                .and_then(|pointer| context.world().pointer_capture(binding.document, pointer.0)),
            route_latency_ns,
        });
        nana_diagnostics::metric!(
            nana_diagnostics::framework::runtime::INPUT_ROUTE_NS,
            route_started.elapsed()
        );
        if let Some(key) = terminal_pointer {
            self.pointers.remove(&key);
        }
        result
    }

    /// Route one event and return the unified outcome required by host
    /// adapters. The legacy [`Self::route`] API remains available for callers
    /// that only need `InputDisposition`.
    pub fn route_with_outcome(
        &mut self,
        context: &mut AppContext,
        event: &nana_ui_platform::CanonicalInputEvent,
        now: Duration,
        text_shaper: Option<&mut dyn TextShaper>,
    ) -> Result<InputRouteOutcome, InputRouterError> {
        let work_before = context.world().pending_work_revision();
        let requests_before = self.host_requests.len();
        let disposition = self.route(context, event, now, text_shaper)?;
        let requests_after = self.host_requests.len();
        Ok(InputRouteOutcome {
            handled: disposition.handled,
            prevent_default: disposition.prevent_default,
            invalidated_work: context.world().pending_work_revision() != work_before,
            host_requests_enqueued: requests_after.saturating_sub(requests_before),
        })
    }

    fn enqueue_ime_request(
        &mut self,
        context: &AppContext,
        event: &nana_ui_platform::CanonicalInputEvent,
        document: DocumentId,
    ) -> Result<(), InputRouterError> {
        let composition = match &event.payload {
            nana_ui_platform::InputPayload::Composition(composition) => composition,
            _ => return Ok(()),
        };
        if matches!(composition, CompositionInput::Enabled)
            && self
                .ime_focus
                .get(&document)
                .copied()
                .is_some_and(|focused| {
                    context
                        .focused_text_input(document)
                        .is_some_and(|(node, _)| node == focused)
                })
        {
            // Focus synchronization already emitted the enable intent for this
            // owner; do not duplicate it for the following composition packet.
            return Ok(());
        }
        let host_context = HostRequestContext {
            source: event.metadata.source,
            generation: event.metadata.generation,
            document: document.get(),
            node: context.world().focused(document).map(StableNodeId::get),
        };
        let surrounding = self.focused_ime_surrounding(context, document);
        let request = match composition {
            CompositionInput::Enabled => HostServiceRequest::ImeEnable {
                context: host_context,
                surrounding,
            },
            CompositionInput::Disabled | CompositionInput::End => HostServiceRequest::ImeDisable {
                context: host_context,
            },
            CompositionInput::Start
            | CompositionInput::Update { .. }
            | CompositionInput::Commit(_)
            | CompositionInput::DeleteSurrounding { .. } => HostServiceRequest::ImeUpdate {
                context: host_context,
                surrounding,
            },
        };
        self.host_requests
            .push(request)
            .map_err(|_| InputRouterError::Dispatch("host service request queue is full".into()))
    }

    fn focused_ime_surrounding(
        &self,
        context: &AppContext,
        document: DocumentId,
    ) -> Option<nana_ui_platform::ImeSurroundingText> {
        let (node, view) = context.focused_text_input(document)?;
        let cursor_area = match context.world().component_geometry(node) {
            Some(nana_ui_runtime::ComponentGeometry::TextInput {
                caret: Some(caret), ..
            }) => Some(caret),
            _ => context.world().layout_box(node),
        }
        .map(|bounds| {
            nana_ui_core::LogicalRect::new(bounds.x, bounds.y, bounds.width, bounds.height)
        });
        Some(nana_ui_platform::ImeSurroundingText {
            text: view.value.to_owned(),
            selection: (
                view.selection.anchor.min(view.selection.focus),
                view.selection.anchor.max(view.selection.focus),
            ),
            cursor_area,
        })
    }

    fn sync_ime_focus_request(
        &mut self,
        context: &AppContext,
        event: &nana_ui_platform::CanonicalInputEvent,
        document: DocumentId,
    ) -> Result<(), InputRouterError> {
        let current = context.focused_text_input(document).map(|(node, _)| node);
        let previous = self.ime_focus.get(&document).copied();
        if previous == current {
            return Ok(());
        }
        let request_context = |node| HostRequestContext {
            source: event.metadata.source,
            generation: event.metadata.generation,
            document: document.get(),
            node,
        };
        if let Some(previous) = previous {
            self.host_requests
                .push(HostServiceRequest::ImeDisable {
                    context: request_context(Some(previous.get())),
                })
                .map_err(|_| {
                    InputRouterError::Dispatch("host service request queue is full".into())
                })?;
        }
        if let Some(current) = current {
            let surrounding = self.focused_ime_surrounding(context, document);
            self.host_requests
                .push(HostServiceRequest::ImeEnable {
                    context: request_context(Some(current.get())),
                    surrounding,
                })
                .map_err(|_| {
                    InputRouterError::Dispatch("host service request queue is full".into())
                })?;
        }
        // Native text input is a capability intent only. The actual text,
        // selection, and caret data continue to come from the existing IME
        // request path, so hosts do not need a second text state machine.
        if !matches!(
            event.payload,
            nana_ui_platform::InputPayload::Composition(_)
        ) {
            self.host_requests
                .push(HostServiceRequest::NativeTextInput {
                    context: request_context(current.map(StableNodeId::get)),
                    enabled: current.is_some(),
                })
                .map_err(|_| {
                    InputRouterError::Dispatch("host service request queue is full".into())
                })?;
        }
        if let Some(current) = current {
            self.ime_focus.insert(document, current);
        } else {
            self.ime_focus.remove(&document);
        }
        Ok(())
    }

    fn sync_cursor_request(
        &mut self,
        context: &AppContext,
        source: nana_ui_platform::InputSourceId,
        device: nana_ui_platform::DeviceId,
        generation: nana_ui_platform::EndpointGeneration,
        document: DocumentId,
        hover: Option<StableNodeId>,
    ) -> Result<(), InputRouterError> {
        let cursor = hover
            .and_then(|node| {
                context.world().computed_style(node).map(|style| {
                    if style.cursor_specified {
                        cursor_spec_name(style.cursor)
                    } else if context.world().text_input(node).is_some() {
                        "text"
                    } else {
                        "default"
                    }
                })
            })
            .unwrap_or("default")
            .to_owned();
        let key = (source, device, document);
        if self.cursor_state.get(&key) == Some(&cursor) {
            return Ok(());
        }
        // Default is the initial host state; do not enqueue a needless intent
        // for the first uncaptured move over empty space.
        if !self.cursor_state.contains_key(&key) && cursor == "default" {
            self.cursor_state.insert(key, cursor);
            return Ok(());
        }
        self.host_requests
            .push(HostServiceRequest::Cursor {
                context: HostRequestContext {
                    source,
                    generation,
                    document: document.get(),
                    node: hover.map(StableNodeId::get),
                },
                cursor: cursor.clone(),
            })
            .map_err(|_| InputRouterError::HostServiceBackpressure)?;
        self.cursor_state.insert(key, cursor);
        Ok(())
    }

    /// Convert canonical clipboard shortcuts into non-blocking host intents.
    /// The legacy adapter keeps its direct API for compatibility, while the
    /// canonical router never waits on an OS clipboard mutex or deletes text
    /// before the host confirms a cut.
    fn route_clipboard_shortcut(
        &mut self,
        context: &AppContext,
        event: &nana_ui_platform::CanonicalInputEvent,
        document: DocumentId,
    ) -> Result<Option<InputDisposition>, InputRouterError> {
        let nana_ui_platform::InputPayload::Key(key) = &event.payload else {
            return Ok(None);
        };
        if key.state != nana_ui_platform::KeyState::Pressed
            || key.modifiers.alt
            || key.modifiers.shift
            || !(key.modifiers.control || key.modifiers.meta)
        {
            return Ok(None);
        }
        let shortcut = key.logical.0.as_ref();
        let cut = match shortcut {
            "c" | "C" => false,
            "x" | "X" => true,
            "v" | "V" => {
                if context.focused_text_input(document).is_none() {
                    return Ok(None);
                }
                false
            }
            _ => return Ok(None),
        };
        let request_context = HostRequestContext {
            source: event.metadata.source,
            generation: event.metadata.generation,
            document: document.get(),
            node: context.world().focused(document).map(StableNodeId::get),
        };
        let request = if matches!(shortcut, "v" | "V") {
            HostServiceRequest::ClipboardRead {
                context: request_context,
            }
        } else {
            let Some(text) = context
                .focused_selected_text(document)
                .or_else(|| context.document_selected_text(document))
            else {
                return Ok(None);
            };
            if text.is_empty() {
                return Ok(None);
            }
            HostServiceRequest::ClipboardWrite {
                context: request_context,
                text,
                cut,
            }
        };
        self.host_requests
            .push(request)
            .map_err(|_| InputRouterError::HostServiceBackpressure)?;
        Ok(Some(InputDisposition {
            handled: true,
            prevent_default: true,
        }))
    }

    /// Drain an event-driven endpoint. No polling or background task is
    /// created; the host calls this when its endpoint has accepted input.
    pub fn route_endpoint(
        &mut self,
        context: &mut AppContext,
        endpoint: &mut nana_ui_platform::InputEndpoint,
        now: Duration,
    ) -> Result<usize, InputRouterError> {
        self.route_endpoint_with_shaper(context, endpoint, now, None)
    }

    pub fn route_endpoint_with_shaper(
        &mut self,
        context: &mut AppContext,
        endpoint: &mut nana_ui_platform::InputEndpoint,
        now: Duration,
        mut text_shaper: Option<&mut dyn TextShaper>,
    ) -> Result<usize, InputRouterError> {
        let mut routed = 0;
        while let Some(event) = endpoint.front() {
            let result = self.route(context, event, now, reborrow_text_shaper(&mut text_shaper));
            if matches!(result, Err(InputRouterError::HostServiceBackpressure)) {
                return result.map(|_| routed);
            }
            // Other failures may follow dispatch or represent permanently stale
            // input. Only pre-dispatch backpressure is safe to retry.
            endpoint.pop();
            result?;
            routed += 1;
        }
        Ok(routed)
    }

    fn remap_pointer(
        &mut self,
        event: &nana_ui_platform::CanonicalInputEvent,
    ) -> nana_ui_platform::CanonicalInputEvent {
        let pointer = match &event.payload {
            nana_ui_platform::InputPayload::Pointer(pointer) => Some(pointer.pointer_id),
            nana_ui_platform::InputPayload::Wheel(wheel) => Some(wheel.pointer_id),
            nana_ui_platform::InputPayload::PointerEnter { pointer_id, .. }
            | nana_ui_platform::InputPayload::PointerLeave { pointer_id } => Some(*pointer_id),
            _ => None,
        };
        let Some(pointer) = pointer else {
            return event.clone();
        };
        let key = (event.metadata.source, event.metadata.device, pointer);
        let pointer_type = match &event.payload {
            nana_ui_platform::InputPayload::Pointer(pointer) => pointer.pointer_type,
            _ => nana_ui_platform::PointerType::Mouse,
        };
        let is_primary = match &event.payload {
            nana_ui_platform::InputPayload::Pointer(pointer) => pointer.is_primary,
            _ => true,
        };
        let local = if let Some(identity) = self.pointers.get_mut(&key) {
            self.counters.pointer_identity_cache_hits += 1;
            // Enter/leave and wheel carry no tool metadata. A later pointer
            // sample supplies it, and metadata-free events must not reset it.
            if let InputPayload::Pointer(pointer) = &event.payload {
                identity.pointer_type = pointer.pointer_type;
                identity.is_primary = pointer.is_primary;
            }
            identity.local
        } else {
            self.counters.pointer_identity_cache_misses += 1;
            let local = self.next_pointer;
            self.next_pointer = self.next_pointer.wrapping_add(1).max(1);
            self.pointers.insert(
                key,
                PointerIdentity {
                    local,
                    pointer_type,
                    is_primary,
                },
            );
            local
        };
        let mut mapped = event.clone();
        match &mut mapped.payload {
            nana_ui_platform::InputPayload::Pointer(pointer) => {
                pointer.pointer_id = nana_ui_platform::PointerId(local)
            }
            nana_ui_platform::InputPayload::Wheel(wheel) => {
                wheel.pointer_id = nana_ui_platform::PointerId(local)
            }
            nana_ui_platform::InputPayload::PointerEnter { pointer_id, .. }
            | nana_ui_platform::InputPayload::PointerLeave { pointer_id } => {
                *pointer_id = nana_ui_platform::PointerId(local)
            }
            _ => {}
        }
        mapped
    }

    fn cancel_captured_pointers(
        &mut self,
        context: &mut AppContext,
        document: DocumentId,
        source: nana_ui_platform::InputSourceId,
        device: Option<nana_ui_platform::DeviceId>,
        now: Duration,
    ) -> Result<(), FrameworkError> {
        let pointers = self
            .pointers
            .iter()
            .filter_map(|(&(bound, candidate_device, _), &identity)| {
                (bound == source && device.is_none_or(|wanted| wanted == candidate_device))
                    .then_some(identity)
            })
            .collect::<Vec<_>>();
        for identity in pointers {
            let pointer = identity.local;
            let (x, y) = context
                .pointer_position(document, pointer)
                .unwrap_or((0.0, 0.0));
            let had_capture = context.world().pointer_capture(document, pointer).is_some();
            let had_press = context.world().pointer_press(document, pointer).is_some();
            if had_capture || had_press {
                // A disconnect/blur is a lifecycle cancellation, not a silent
                // state reset: components must receive the same cancel path as
                // an explicit pointer cancel before capture is revoked.
                self.adapter.dispatch_with_shaper(
                    context,
                    document,
                    &InputEvent::Pointer {
                        phase: PointerPhase::Cancel,
                        pointer_id: pointer,
                        pointer_type: identity.pointer_type,
                        x,
                        y,
                        screen_x: x,
                        screen_y: y,
                        button: -1,
                        buttons: 0,
                        pressure: 0.0,
                        tangential_pressure: 0.0,
                        tilt_x: 0,
                        tilt_y: 0,
                        twist: 0,
                        is_primary: identity.is_primary,
                        activation_click: false,
                        modifiers: Default::default(),
                    },
                    now,
                    None,
                )?;
            }
            context.release_pointer_capture(document, pointer);
            context.release_pointer(document, pointer);
            context.set_pointer_location(document, pointer, None);
            context.set_pointer_hover_at(document, pointer, None, now)?;
        }
        self.pointers.retain(|(bound, candidate_device, _), _| {
            bound != &source || device.is_some_and(|wanted| *candidate_device != wanted)
        });
        Ok(())
    }
}

impl std::fmt::Debug for RuntimeInputAdapter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RuntimeInputAdapter")
            .field("line_scroll_extent", &self.line_scroll_extent)
            .field("table_page_rows", &self.table_page_rows)
            .field("own_clipboard", &self.clipboard.is_some())
            .finish()
    }
}

impl Default for RuntimeInputAdapter {
    fn default() -> Self {
        Self {
            line_scroll_extent: DEFAULT_LINE_SCROLL_EXTENT,
            table_page_rows: 10,
            clipboard: None,
        }
    }
}

impl RuntimeInputAdapter {
    /// Route one canonical event through the existing Runtime interaction
    /// authority. Hosts may inject this path without constructing a window
    /// event; the legacy `InputEvent` conversion is kept local to this adapter
    /// until all platform adapters have migrated.
    pub fn dispatch_canonical(
        &mut self,
        context: &mut AppContext,
        document: DocumentId,
        event: &CanonicalInputEvent,
        now: Duration,
        text_shaper: Option<&mut dyn TextShaper>,
    ) -> Result<InputDisposition, FrameworkError> {
        let payload = &event.payload;
        match payload {
            InputPayload::Pointer(pointer) => self.dispatch_with_shaper(
                context,
                document,
                &InputEvent::Pointer {
                    phase: pointer.phase,
                    pointer_id: pointer.pointer_id.0,
                    pointer_type: pointer.pointer_type,
                    x: pointer.x,
                    y: pointer.y,
                    screen_x: pointer.screen_x,
                    screen_y: pointer.screen_y,
                    button: pointer.button,
                    buttons: pointer.buttons,
                    pressure: pointer.pressure,
                    tangential_pressure: pointer.tangential_pressure,
                    tilt_x: pointer.tilt_x,
                    tilt_y: pointer.tilt_y,
                    twist: pointer.twist,
                    is_primary: pointer.is_primary,
                    activation_click: pointer.activation_click,
                    modifiers: pointer.modifiers,
                },
                now,
                text_shaper,
            ),
            InputPayload::Wheel(wheel) => self.dispatch_with_shaper(
                context,
                document,
                &InputEvent::Wheel {
                    x: wheel.x,
                    y: wheel.y,
                    delta_x: wheel.delta_x,
                    delta_y: wheel.delta_y,
                    line_delta: wheel.unit == nana_ui_platform::WheelUnit::Lines,
                    modifiers: wheel.modifiers,
                },
                now,
                text_shaper,
            ),
            InputPayload::Key(key) => self.dispatch_with_shaper(
                context,
                document,
                &InputEvent::Keyboard {
                    pressed: key.state == nana_ui_platform::KeyState::Pressed,
                    key: key.logical.0.to_string(),
                    text: None,
                    code: key.physical.0.to_string(),
                    repeat: key.repeat,
                    modifiers: key.modifiers,
                },
                now,
                text_shaper,
            ),
            InputPayload::TextInput(text) => {
                if text.is_empty() {
                    return Ok(InputDisposition::default());
                }
                // Keep committed text on the same path as a native keyboard
                // event. This preserves overlay barriers, terminal key
                // handling, editor pairing/completion and text shaping while
                // retaining the canonical contract that text is not inferred
                // from a key transition.
                self.dispatch_with_shaper(
                    context,
                    document,
                    &InputEvent::Keyboard {
                        pressed: true,
                        key: String::new(),
                        text: Some(text.clone()),
                        code: String::new(),
                        repeat: false,
                        modifiers: nana_ui_platform::InputModifiers::default(),
                    },
                    now,
                    text_shaper,
                )
            }
            InputPayload::Composition(composition) => {
                let ime = match composition {
                    CompositionInput::Enabled => ImeEvent::Enabled,
                    CompositionInput::Disabled => ImeEvent::Disabled,
                    CompositionInput::Start => ImeEvent::Preedit {
                        text: String::new(),
                        selection: None,
                    },
                    CompositionInput::Update { text, selection } => ImeEvent::Preedit {
                        text: text.clone(),
                        selection: *selection,
                    },
                    CompositionInput::Commit(text) => ImeEvent::Commit(text.clone()),
                    CompositionInput::End => ImeEvent::Cancelled,
                    CompositionInput::DeleteSurrounding {
                        before_bytes,
                        after_bytes,
                    } => ImeEvent::DeleteSurrounding {
                        before_bytes: *before_bytes,
                        after_bytes: *after_bytes,
                    },
                };
                self.dispatch_ime(context, document, &ime)
            }
            InputPayload::PointerEnter { pointer_id, x, y } => {
                let target = context.world().hit_test(document, *x, *y);
                context.set_pointer_location(document, pointer_id.0, Some((*x, *y)));
                context.set_pointer_hover_at(document, pointer_id.0, target, now)?;
                Ok(InputDisposition::default())
            }
            InputPayload::PointerLeave { pointer_id } => {
                context.set_pointer_location(document, pointer_id.0, None);
                context.set_pointer_hover_at(document, pointer_id.0, None, now)?;
                Ok(InputDisposition::default())
            }
            InputPayload::Focus { .. }
            | InputPayload::DeviceConnected
            | InputPayload::DeviceDisconnected
            | InputPayload::SourceConnected
            | InputPayload::SourceDisconnected => Ok(InputDisposition::default()),
        }
    }

    /// Route clipboard shortcuts through `clipboard` instead of the process
    /// pasteboard. Hosts with their own backend, and tests, install one here.
    #[must_use]
    pub fn with_clipboard(mut self, clipboard: SharedClipboardHost) -> Self {
        self.clipboard = Some(clipboard);
        self
    }

    fn clipboard(&self) -> &SharedClipboardHost {
        match &self.clipboard {
            Some(clipboard) => clipboard,
            None => process_clipboard(),
        }
    }

    fn read_clipboard(&self) -> Option<String> {
        self.clipboard()
            // Clipboard is a host capability, not part of the input critical
            // section. Never wait for an OS/JNI clipboard call here.
            .try_lock()
            .ok()?
            .read_text()
            .filter(|text| !text.is_empty())
    }

    fn write_clipboard(&self, text: &str) -> bool {
        self.clipboard()
            .try_lock()
            .is_ok_and(|mut clipboard| clipboard.write_text(text))
    }

    pub fn dispatch(
        &mut self,
        context: &mut AppContext,
        document: DocumentId,
        event: &InputEvent,
    ) -> Result<InputDisposition, FrameworkError> {
        self.dispatch_with_shaper(context, document, event, Duration::ZERO, None)
    }

    /// Dispatch input at the host's monotonic Runtime timestamp. Timed
    /// component behavior such as tooltip delay uses this clock; no component
    /// owns a timer or requests frames while idle.
    pub fn dispatch_at(
        &mut self,
        context: &mut AppContext,
        document: DocumentId,
        event: &InputEvent,
        now: Duration,
    ) -> Result<InputDisposition, FrameworkError> {
        self.dispatch_with_shaper(context, document, event, now, None)
    }

    /// Dispatch input with the host text shaper so caret movement,
    /// click-to-caret, and drag selection follow real text geometry. Hosts
    /// without a shaper fall back to logical-line caret movement.
    pub fn dispatch_with_shaper(
        &mut self,
        context: &mut AppContext,
        document: DocumentId,
        event: &InputEvent,
        now: Duration,
        text_shaper: Option<&mut dyn TextShaper>,
    ) -> Result<InputDisposition, FrameworkError> {
        let mut text_shaper = text_shaper;
        // Before anything routes, so the focus this event causes is recorded
        // against the device that caused it. A pointer press focuses the node
        // it hit well before it records the press itself, which is why this
        // cannot be inferred downstream.
        match event {
            InputEvent::Pointer { .. } => context
                .compat_world_mut()
                .note_input_modality(document, nana_ui_runtime::InputModality::Pointer),
            InputEvent::Keyboard { .. } => context
                .compat_world_mut()
                .note_input_modality(document, nana_ui_runtime::InputModality::Keyboard),
            // A wheel moves nothing into focus; leave the answer alone.
            InputEvent::Wheel { .. } => {}
        }
        let keyboard_barrier = matches!(event, InputEvent::Keyboard { .. })
            && context.has_blocking_runtime_overlay(document);
        if let InputEvent::Keyboard {
            pressed: true,
            key,
            repeat,
            modifiers,
            ..
        } = event
            && !modifiers.alt
            && !modifiers.control
            && !modifiers.meta
        {
            let overlay_key = match key.as_str() {
                "Escape" if !repeat => Some(OverlayKey::Escape),
                "Tab" => Some(OverlayKey::Tab {
                    reverse: modifiers.shift,
                }),
                _ => None,
            };
            if matches!(overlay_key, Some(OverlayKey::Escape))
                && !modifiers.shift
                && context.dismiss_focused_field_options(document)?
            {
                return Ok(InputDisposition {
                    handled: true,
                    prevent_default: true,
                });
            }
            if let Some(key) = overlay_key
                && context.route_overlay_key(document, key)?
            {
                return Ok(InputDisposition {
                    handled: true,
                    prevent_default: true,
                });
            }
            if matches!(overlay_key, Some(OverlayKey::Escape))
                && context.dismiss_popovers_on_escape()?
            {
                return Ok(InputDisposition {
                    handled: true,
                    prevent_default: true,
                });
            }
            // overlay 未消费的 Esc：先取消拖拽移动选中，再结束 snippet 会
            // 话，签名帮助在场时消费但不关补全（两段式：宿主随后撤签名），
            // 再关闭补全弹层，最后塌缩多光标到主光标。都只在聚焦多行
            // 编辑器且状态存在时消费事件，否则穿透给宿主（首次按下才生
            // 效，repeat 不消费）。
            if matches!(overlay_key, Some(OverlayKey::Escape))
                && !repeat
                && (context.cancel_focused_text_selection_drag(document)
                    || context.cancel_focused_text_snippet(document)?
                    || context.focused_text_signature_showing(document)
                    || context.dismiss_focused_text_completion(document)?
                    || context.collapse_focused_text_selections(document)?)
            {
                return Ok(InputDisposition {
                    handled: true,
                    prevent_default: true,
                });
            }
        }
        if let InputEvent::Keyboard {
            pressed,
            key,
            repeat,
            modifiers,
            ..
        } = event
            && !keyboard_barrier
            && context.dispatch_focused_key(
                document,
                &nana_ui_runtime::KeyInput::new(
                    *pressed,
                    key,
                    modifiers.alt,
                    modifiers.control,
                    modifiers.shift,
                    modifiers.meta,
                    *repeat,
                ),
            )
        {
            return Ok(InputDisposition {
                handled: true,
                prevent_default: true,
            });
        }
        if let InputEvent::Keyboard {
            pressed: true,
            key,
            text,
            modifiers,
            ..
        } = event
            && !keyboard_barrier
            && context.focused_terminal(document).is_some()
        {
            if (modifiers.control && modifiers.shift || modifiers.meta)
                && key.eq_ignore_ascii_case("c")
            {
                if let Some(text) = context
                    .terminal_selected_text(document)
                    .filter(|text| !text.is_empty())
                {
                    self.write_clipboard(&text);
                }
            } else if (modifiers.control && modifiers.shift || modifiers.meta)
                && key.eq_ignore_ascii_case("v")
            {
                if let Some(text) = self.read_clipboard() {
                    context.paste_terminal(document, &text)?;
                }
            } else {
                context.terminal_key(
                    document,
                    key,
                    text.as_deref(),
                    modifiers.control,
                    modifiers.alt,
                    modifiers.shift,
                )?;
            }
            return Ok(InputDisposition {
                handled: true,
                prevent_default: true,
            });
        }
        // Focused plain text editors own their editing keys (caret moves,
        // selection, deletion, indent, pairing) before any generic routing.
        if let InputEvent::Keyboard {
            pressed: true,
            key,
            text,
            repeat: _,
            modifiers,
            ..
        } = event
            && Self::text_editor_key(
                context,
                document,
                key,
                text.as_deref(),
                *modifiers,
                reborrow_text_shaper(&mut text_shaper),
            )?
        {
            return Ok(InputDisposition {
                handled: true,
                prevent_default: true,
            });
        }
        if let InputEvent::Keyboard {
            pressed: true,
            key,
            repeat,
            modifiers,
            ..
        } = event
        {
            if key == "Tab"
                && !modifiers.alt
                && !modifiers.control
                && !modifiers.meta
                && context.navigate_sequential_focus(document, modifiers.shift)?
            {
                return Ok(InputDisposition {
                    handled: true,
                    prevent_default: true,
                });
            }
            if !modifiers.alt && !modifiers.control && !modifiers.meta && !modifiers.shift {
                let segmented_navigation = match key.as_str() {
                    "ArrowLeft" => Some(RovingFocusIntent::Previous),
                    "ArrowRight" => Some(RovingFocusIntent::Next),
                    "Home" => Some(RovingFocusIntent::First),
                    "End" => Some(RovingFocusIntent::Last),
                    _ => None,
                };
                if let Some(intent) = segmented_navigation
                    && context.navigate_focused_segmented(document, intent)?
                {
                    return Ok(InputDisposition {
                        handled: true,
                        prevent_default: true,
                    });
                }
                if matches!(key.as_str(), " " | "Space" | "Enter")
                    && let Some(target) = context.world().focused(document)
                    && context.is_segmented_option_node(target)
                {
                    if !repeat {
                        context.activate_node(target)?;
                    }
                    return Ok(InputDisposition {
                        handled: true,
                        prevent_default: true,
                    });
                }
            }
        }
        let handled = match event {
            InputEvent::Pointer {
                phase,
                pointer_id,
                x,
                y,
                button,
                is_primary,
                activation_click,
                modifiers,
                ..
            } => {
                let overlay_phase = match phase {
                    PointerPhase::Move => OverlayPointerPhase::Move,
                    PointerPhase::Down if *is_primary && *button == 0 => {
                        OverlayPointerPhase::PrimaryDown
                    }
                    PointerPhase::Up if *is_primary && *button == 0 => {
                        OverlayPointerPhase::PrimaryUp
                    }
                    PointerPhase::Cancel => OverlayPointerPhase::Cancel,
                    PointerPhase::Down | PointerPhase::Up => OverlayPointerPhase::Move,
                };
                let overlay = if matches!(
                    phase,
                    PointerPhase::Move | PointerPhase::Up | PointerPhase::Cancel
                ) {
                    context
                        .world()
                        .pointer_capture(document, *pointer_id)
                        .map_or_else(
                            || {
                                context.route_overlay_pointer(
                                    document,
                                    *pointer_id,
                                    overlay_phase,
                                    *x,
                                    *y,
                                )
                            },
                            |target| {
                                Ok(OverlayPointerDecision {
                                    target: Some(target),
                                    prevent_default: false,
                                    dismissed: false,
                                })
                            },
                        )?
                } else {
                    context.route_overlay_pointer(document, *pointer_id, overlay_phase, *x, *y)?
                };
                let target = overlay.target;
                context.set_pointer_location(document, *pointer_id, Some((*x, *y)));
                context.set_pointer_hover_at(document, *pointer_id, target, now)?;
                context.update_text_diagnostic_hover(document, *x, *y)?;
                let terminal_phase = match phase {
                    PointerPhase::Down if *is_primary && *button == 0 => Some(0),
                    PointerPhase::Move => Some(1),
                    PointerPhase::Up if *is_primary && *button == 0 => Some(2),
                    PointerPhase::Cancel => Some(3),
                    _ => None,
                };
                if !overlay.prevent_default
                    && let Some(phase) = terminal_phase
                    && context.terminal_pointer(document, target, *pointer_id, phase, *x, *y)?
                {
                    return Ok(InputDisposition {
                        handled: true,
                        prevent_default: true,
                    });
                }

                #[cfg(feature = "graph-canvas")]
                let graph_button = match *button {
                    1 => GraphPointerButton::Middle,
                    _ => GraphPointerButton::Primary,
                };
                let component_handled = match phase {
                    PointerPhase::Move => {
                        if context.update_text_area_resize(document, *pointer_id, *x, *y)? {
                            return Ok(InputDisposition {
                                handled: true,
                                prevent_default: true,
                            });
                        }
                        if optional_input!(
                            "rich-text",
                            context.update_rich_text_pointer(document, *pointer_id, *x, *y),
                            Ok::<bool, FrameworkError>(false)
                        )? {
                            return Ok(InputDisposition {
                                handled: true,
                                prevent_default: true,
                            });
                        }
                        #[cfg(feature = "image-viewer")]
                        if let Some(viewer) = context
                            .world()
                            .pointer_capture(document, *pointer_id)
                            .and_then(|target| {
                                context.view_entity::<nana_ui_runtime::ImageViewer>(target)
                            })
                            && context.image_viewer_pointer_move(viewer, *pointer_id, *x, *y)?
                        {
                            return Ok(InputDisposition {
                                handled: true,
                                prevent_default: true,
                            });
                        }
                        if let Some(shaper) = reborrow_text_shaper(&mut text_shaper)
                            && context.text_editor_pointer_drag(
                                document,
                                *pointer_id,
                                *x,
                                *y,
                                shaper,
                            )?
                        {
                            true
                        } else if let Some(shaper) = reborrow_text_shaper(&mut text_shaper)
                            && context.document_text_pointer_drag(
                                document,
                                *pointer_id,
                                *x,
                                *y,
                                shaper,
                            )?
                        {
                            true
                        } else {
                            context.update_scrollbar_drag(document, *pointer_id, *x, *y)?
                                || context.update_range_drag(document, *pointer_id, *x)?
                                || context.update_xy_pad_drag(
                                    document,
                                    *pointer_id,
                                    *x,
                                    *y,
                                    modifiers.shift,
                                )?
                                || optional_input!(
                                    "graph-canvas",
                                    context.update_graph_canvas_pointer(
                                        document,
                                        *pointer_id,
                                        *x,
                                        *y,
                                    ),
                                    Ok::<bool, FrameworkError>(false)
                                )?
                                || optional_input!(
                                    "graph-canvas",
                                    context.update_graph_minimap_pointer(
                                        document,
                                        *pointer_id,
                                        *x,
                                        *y,
                                    ),
                                    Ok::<bool, FrameworkError>(false)
                                )?
                                || optional_input!(
                                    "controls",
                                    context.update_reorder_list_pointer(
                                        document,
                                        *pointer_id,
                                        *x,
                                        *y,
                                    ),
                                    Ok::<bool, FrameworkError>(false)
                                )?
                                || context.update_split_resize(document, *pointer_id, *x, *y)?
                                || context.update_dock_split_resize(
                                    document,
                                    *pointer_id,
                                    *x,
                                    *y,
                                )?
                                || context.update_workspace_resize(
                                    document,
                                    *pointer_id,
                                    *x,
                                    *y,
                                    now,
                                )?
                                || context.update_dock_item_drag(document, *pointer_id, *x, *y)?
                                || target
                                    .map(|_target| {
                                        optional_input!(
                                            "graph-canvas",
                                            context.hover_graph_canvas(_target, *x, *y),
                                            Ok::<bool, FrameworkError>(false)
                                        )
                                    })
                                    .transpose()?
                                    .unwrap_or(false)
                                || target
                                    .map(|_target| {
                                        optional_input!(
                                            "calendar",
                                            context.hover_calendar_heatmap(_target, *x, *y),
                                            Ok::<bool, FrameworkError>(false)
                                        )
                                    })
                                    .transpose()?
                                    .unwrap_or(false)
                                || optional_input!(
                                    "calendar",
                                    context.clear_calendar_heatmap_hover(document),
                                    Ok::<bool, FrameworkError>(false)
                                )?
                                || context.sync_split_handle_hover_near(document, *x, *y, now)?
                                || target.is_some()
                        }
                    }
                    PointerPhase::Down if *button == 2 => {
                        // A secondary press outside an open popover dismisses
                        // it and goes no further, matching the primary press.
                        if context.dismiss_popovers_outside(target)? {
                            return Ok(InputDisposition {
                                handled: true,
                                prevent_default: true,
                            });
                        }
                        context.dismiss_detached_menus(target)?;
                        context.secondary_press_at(document, *x, *y)?.is_some()
                    }
                    PointerPhase::Down if (*is_primary && *button == 0) || *button == 1 => {
                        if context.dismiss_popovers_outside(target)? {
                            // Consume the press that dismissed the popover.
                            // Activation needs a press recorded here to match
                            // on release, so skipping it also stops this click
                            // from reaching the control underneath.
                            return Ok(InputDisposition {
                                handled: true,
                                prevent_default: true,
                            });
                        }
                        context.dismiss_detached_menus(target)?;
                        // Scrollbars overlay content, so they claim the press
                        // before the node underneath sees it.
                        if *button == 0
                            && !activation_click
                            && let Some(target) = target
                            && context.begin_text_area_resize(*pointer_id, target, *x, *y)?
                        {
                            return Ok(InputDisposition {
                                handled: true,
                                prevent_default: true,
                            });
                        }
                        if *button == 0
                            && let Some((view, axis)) =
                                context.scrollbar_target_near(document, *x, *y)
                            && context.begin_scrollbar_drag(*pointer_id, view, axis, *x, *y)?
                        {
                            return Ok(InputDisposition {
                                handled: true,
                                prevent_default: true,
                            });
                        }
                        let split_handle = context.split_handle_near(document, *x, *y);
                        let dock_handle = context.dock_handle_near(document, *x, *y);
                        let workspace_handle = context.workspace_handle_near(document, *x, *y);
                        let dock_source = target
                            .filter(|id| context.is_dock_item_source(*id))
                            .or_else(|| context.dock_tab_strip_near(document, *x, *y));
                        let hit = dock_handle.or(split_handle).or(workspace_handle).or(target);
                        let focus_target = hit.and_then(|id| nearest_focusable(context, id));
                        if !hit.is_some_and(|id| context.preserves_hover_card_editor_focus(id)) {
                            if let Some(focus) = focus_target {
                                context.focus_node(document, focus)?;
                            } else {
                                context.clear_focus(document)?;
                            }
                        }
                        if *button == 0
                            && !activation_click
                            && let Some(shaper) = reborrow_text_shaper(&mut text_shaper)
                        {
                            let editor = if let Some(focus) = focus_target {
                                context.text_editor_pointer_press(
                                    document,
                                    focus,
                                    *pointer_id,
                                    *x,
                                    *y,
                                    modifiers.shift,
                                    modifiers.alt,
                                    now,
                                    shaper,
                                )?
                            } else {
                                false
                            };
                            if editor {
                                context.clear_document_text_selection(document);
                            } else {
                                context.document_text_pointer_press(
                                    document,
                                    *pointer_id,
                                    *x,
                                    *y,
                                    shaper,
                                )?;
                            }
                        }
                        if let Some(target) = hit {
                            if *button == 0
                                && !activation_click
                                && optional_input!(
                                    "rich-text",
                                    context.begin_rich_text_pointer(
                                        document,
                                        *pointer_id,
                                        target,
                                        *x,
                                        *y
                                    ),
                                    Ok::<bool, FrameworkError>(false)
                                )?
                            {
                                return Ok(InputDisposition {
                                    handled: true,
                                    prevent_default: true,
                                });
                            }
                            #[cfg(feature = "image-viewer")]
                            if *button == 0
                                && !activation_click
                                && let Some(viewer) =
                                    context.view_entity::<nana_ui_runtime::ImageViewer>(target)
                                && context
                                    .image_viewer_pointer_down(viewer, *pointer_id, *x, *y)?
                                    .is_some()
                            {
                                return Ok(InputDisposition {
                                    handled: true,
                                    prevent_default: true,
                                });
                            }
                            if optional_input!(
                                "graph-canvas",
                                context.is_graph_canvas(target),
                                false
                            ) {
                                optional_input!(
                                    "graph-canvas",
                                    context.begin_graph_canvas_pointer(
                                        document,
                                        *pointer_id,
                                        target,
                                        *x,
                                        *y,
                                        graph_button,
                                    ),
                                    Ok::<bool, FrameworkError>(false)
                                )?;
                            } else if optional_input!(
                                "graph-canvas",
                                context.is_graph_minimap(target),
                                false
                            ) {
                                optional_input!(
                                    "graph-canvas",
                                    context.begin_graph_minimap_pointer(
                                        *pointer_id,
                                        target,
                                        *x,
                                        *y
                                    ),
                                    Ok::<bool, FrameworkError>(false)
                                )?;
                            } else if *button == 0
                                && optional_input!(
                                    "controls",
                                    context.begin_reorder_list_pointer(
                                        document,
                                        *pointer_id,
                                        target,
                                        *x,
                                        *y,
                                    ),
                                    Ok::<bool, FrameworkError>(false)
                                )?
                            {
                            } else if context.is_dock_handle(target) && *button == 0 {
                                context.begin_dock_split_resize(
                                    document,
                                    *pointer_id,
                                    target,
                                    *x,
                                    *y,
                                )?;
                            } else if context.is_split_handle(target) && *button == 0 {
                                context.begin_split_resize(
                                    document,
                                    *pointer_id,
                                    target,
                                    *x,
                                    *y,
                                )?;
                            } else if context.is_workspace_resize_handle(target) && *button == 0 {
                                context.begin_workspace_resize(
                                    document,
                                    *pointer_id,
                                    target,
                                    *x,
                                    *y,
                                    now,
                                )?;
                            } else if *button == 0 {
                                if let Some(source) = dock_source {
                                    context.begin_dock_item_drag(
                                        document,
                                        *pointer_id,
                                        source,
                                        *x,
                                        *y,
                                    )?;
                                } else {
                                    context.press_pointer(document, *pointer_id, target)?;
                                    if !*activation_click
                                        && context.press_number_stepper(target, *x, *y)?
                                    {
                                        context.release_pointer(document, *pointer_id);
                                    } else if context.is_range_field(target) {
                                        context.begin_range_drag(
                                            document,
                                            *pointer_id,
                                            target,
                                            *x,
                                        )?;
                                    } else if context.is_xy_pad(target) {
                                        context.begin_xy_pad_drag(
                                            document,
                                            *pointer_id,
                                            target,
                                            *x,
                                            *y,
                                        )?;
                                    }
                                }
                            }
                            true
                        } else {
                            false
                        }
                    }
                    PointerPhase::Up if (*is_primary && *button == 0) || *button == 1 => {
                        if context.end_text_area_resize(document, *pointer_id, false)? {
                            return Ok(InputDisposition {
                                handled: true,
                                prevent_default: true,
                            });
                        }
                        if optional_input!(
                            "rich-text",
                            context.end_rich_text_pointer(document, *pointer_id, *x, *y, false),
                            Ok::<bool, FrameworkError>(false)
                        )? {
                            return Ok(InputDisposition {
                                handled: true,
                                prevent_default: true,
                            });
                        }
                        #[cfg(feature = "image-viewer")]
                        if let Some(viewer) = context
                            .world()
                            .pointer_capture(document, *pointer_id)
                            .and_then(|target| {
                                context.view_entity::<nana_ui_runtime::ImageViewer>(target)
                            })
                            && context.image_viewer_pointer_up(viewer, *pointer_id)?
                        {
                            return Ok(InputDisposition {
                                handled: true,
                                prevent_default: true,
                            });
                        }
                        // 拖拽移动选中的落点执行先于通用释放清理：active 态
                        // 落文本、pending 态回落为点击。
                        let mut drop_handled = false;
                        if let Some(shaper) = reborrow_text_shaper(&mut text_shaper) {
                            drop_handled = context.text_editor_selection_drop(
                                document,
                                *pointer_id,
                                *x,
                                *y,
                                shaper,
                            )?;
                        }
                        context.text_editor_pointer_release(*pointer_id);
                        context.document_text_pointer_release(*pointer_id);
                        if drop_handled {
                            context.release_pointer(document, *pointer_id);
                            return Ok(InputDisposition {
                                handled: true,
                                prevent_default: true,
                            });
                        }
                        if context.end_scrollbar_drag(document, *pointer_id, false)?
                            || context.end_range_drag(document, *pointer_id, false)?
                            || context.end_xy_pad_drag(document, *pointer_id, false)?
                            || optional_input!(
                                "graph-canvas",
                                context.end_graph_canvas_pointer(
                                    document,
                                    *pointer_id,
                                    *x,
                                    *y,
                                    false,
                                ),
                                Ok::<bool, FrameworkError>(false)
                            )?
                            || optional_input!(
                                "graph-canvas",
                                context.end_graph_minimap_pointer(document, *pointer_id, false),
                                Ok::<bool, FrameworkError>(false)
                            )?
                            || optional_input!(
                                "controls",
                                context.end_reorder_list_pointer(
                                    document,
                                    *pointer_id,
                                    *x,
                                    *y,
                                    false,
                                ),
                                Ok::<bool, FrameworkError>(false)
                            )?
                            || context.end_split_resize(document, *pointer_id, false)?
                            || context.end_dock_split_resize(document, *pointer_id, false)?
                            || context.end_workspace_resize(document, *pointer_id, now)?
                            || context.end_dock_item_drag(document, *pointer_id, *x, *y, false)?
                        {
                            context.release_pointer(document, *pointer_id);
                            return Ok(InputDisposition {
                                handled: true,
                                prevent_default: true,
                            });
                        }
                        let pressed = context.release_pointer(document, *pointer_id);
                        if let Some(pressed) = pressed {
                            if Some(pressed) == target && !*activation_click {
                                context.activate_node_at(pressed, *x, *y)?;
                            }
                            true
                        } else {
                            false
                        }
                    }
                    PointerPhase::Cancel => {
                        context.end_text_area_resize(document, *pointer_id, true)?;
                        optional_input!(
                            "rich-text",
                            context.end_rich_text_pointer(document, *pointer_id, *x, *y, true),
                            Ok::<bool, FrameworkError>(false)
                        )?;
                        #[cfg(feature = "image-viewer")]
                        if let Some(viewer) = context
                            .world()
                            .pointer_capture(document, *pointer_id)
                            .and_then(|target| {
                                context.view_entity::<nana_ui_runtime::ImageViewer>(target)
                            })
                        {
                            context.image_viewer_pointer_up(viewer, *pointer_id)?;
                        }
                        context.text_editor_pointer_release(*pointer_id);
                        context.document_text_pointer_release(*pointer_id);
                        let scrollbar = context.end_scrollbar_drag(document, *pointer_id, true)?;
                        let range = context.end_range_drag(document, *pointer_id, true)?;
                        let xy_pad = context.end_xy_pad_drag(document, *pointer_id, true)?;
                        let graph = optional_input!(
                            "graph-canvas",
                            context.end_graph_canvas_pointer(document, *pointer_id, *x, *y, true,),
                            Ok::<bool, FrameworkError>(false)
                        )?;
                        let minimap = optional_input!(
                            "graph-canvas",
                            context.end_graph_minimap_pointer(document, *pointer_id, true),
                            Ok::<bool, FrameworkError>(false)
                        )?;
                        let reorder = optional_input!(
                            "controls",
                            context.end_reorder_list_pointer(document, *pointer_id, *x, *y, true,),
                            Ok::<bool, FrameworkError>(false)
                        )?;
                        let split = context.end_split_resize(document, *pointer_id, true)?;
                        let dock_split =
                            context.end_dock_split_resize(document, *pointer_id, true)?;
                        let workspace = context.end_workspace_resize(document, *pointer_id, now)?;
                        let dock_item =
                            context.end_dock_item_drag(document, *pointer_id, *x, *y, true)?;
                        let pressed = context.release_pointer(document, *pointer_id).is_some();
                        context.set_pointer_hover_at(document, *pointer_id, None, now)?;
                        let calendar = optional_input!(
                            "calendar",
                            context.clear_calendar_heatmap_hover(document),
                            Ok::<bool, FrameworkError>(false)
                        )?;
                        let split_hover = context.sync_split_handle_hover(document, None)?;
                        scrollbar
                            || range
                            || xy_pad
                            || graph
                            || minimap
                            || reorder
                            || split
                            || dock_split
                            || workspace
                            || dock_item
                            || calendar
                            || split_hover
                            || pressed
                    }
                    _ => false,
                };
                overlay.prevent_default || component_handled
            }
            InputEvent::Wheel {
                x,
                y,
                delta_x,
                delta_y,
                line_delta,
                modifiers,
            } => {
                let (dx, dy) = if modifiers.shift && !cfg!(target_os = "macos") {
                    (*delta_y, *delta_x)
                } else {
                    (*delta_x, *delta_y)
                };
                let scale = if *line_delta {
                    self.line_scroll_extent
                } else {
                    1.0
                };
                let delta = ScrollOffset {
                    x: -dx * scale,
                    y: -dy * scale,
                };
                let overlay = context.route_overlay_pointer(
                    document,
                    0,
                    OverlayPointerPhase::Wheel,
                    *x,
                    *y,
                )?;
                // 锚定浮层（补全弹层 / hover 浮窗）优先：指针落在浮层面板
                #[cfg(feature = "image-viewer")]
                if let Some(viewer) = overlay
                    .target
                    .or_else(|| context.pointer_target(document, *x, *y))
                    .and_then(|target| context.view_entity::<nana_ui_runtime::ImageViewer>(target))
                    && context.image_viewer_wheel(viewer, *x, *y, dy)?
                {
                    return Ok(InputDisposition {
                        handled: true,
                        prevent_default: true,
                    });
                }
                // 上时滚轮滚动浮层自身（按行，方向跟随滚轮），不再落到
                // 编辑器或文档滚动。
                let overlay_rows = if *delta_y > 0.0 {
                    1isize
                } else if *delta_y < 0.0 {
                    -1
                } else {
                    0
                };
                if overlay_rows != 0
                    && context.scroll_text_overlay_at(document, *x, *y, overlay_rows)?
                {
                    return Ok(InputDisposition {
                        handled: true,
                        prevent_default: true,
                    });
                }
                #[cfg(feature = "graph-canvas")]
                let graph_delta = if *line_delta {
                    GraphScrollDelta::Lines { y: -dy }
                } else {
                    GraphScrollDelta::Pixels { y: -dy }
                };
                let graph_target = context.pointer_target(document, *x, *y);
                let scrolled = if overlay.prevent_default {
                    overlay
                        .target
                        .map(|target| context.scroll_overlay_from(document, target, delta))
                        .transpose()?
                        .flatten()
                        .is_some()
                } else if graph_target.is_some_and(|_target| {
                    optional_input!("graph-canvas", context.is_graph_canvas(_target), false)
                }) {
                    optional_input!(
                        "graph-canvas",
                        context.scroll_graph_canvas(
                            document,
                            graph_target.expect("graph target"),
                            *x,
                            *y,
                            graph_delta,
                        ),
                        Ok::<bool, FrameworkError>(false)
                    )?
                } else {
                    context.scroll_at(document, *x, *y, delta)?.is_some()
                };
                overlay.prevent_default || scrolled
            }
            InputEvent::Keyboard {
                pressed,
                key,
                text,
                repeat: _,
                modifiers,
                ..
            } if *pressed
                && !modifiers.alt
                && (modifiers.control || modifiers.meta)
                && key.eq_ignore_ascii_case("z") =>
            {
                // Undo/redo is the one editing shortcut that carries Shift, so
                // it is matched before the clipboard arm excludes it.
                if modifiers.shift {
                    context.redo_focused_text(document)?
                } else {
                    context.undo_focused_text(document)?
                }
            }
            InputEvent::Keyboard {
                pressed,
                key,
                text,
                repeat: _,
                modifiers,
                ..
            } if *pressed && !modifiers.alt && !modifiers.shift => {
                let primary = modifiers.control || modifiers.meta;
                if primary && self.dispatch_clipboard_shortcut(context, document, key)? {
                    return Ok(InputDisposition {
                        handled: true,
                        prevent_default: true,
                    });
                }
                let range_adjustment = (!primary)
                    .then_some(match key.as_str() {
                        "ArrowLeft" | "ArrowDown" => Some(RangeAdjustment::Decrement),
                        "ArrowRight" | "ArrowUp" => Some(RangeAdjustment::Increment),
                        "PageDown" => Some(RangeAdjustment::PageDecrement),
                        "PageUp" => Some(RangeAdjustment::PageIncrement),
                        "Home" => Some(RangeAdjustment::Minimum),
                        "End" => Some(RangeAdjustment::Maximum),
                        _ => None,
                    })
                    .flatten();
                if let Some(adjustment) = range_adjustment
                    && context.adjust_focused_range(document, adjustment)?
                {
                    return Ok(InputDisposition {
                        handled: true,
                        prevent_default: true,
                    });
                }
                let xy_adjustment = (!primary)
                    .then_some(match key.as_str() {
                        "ArrowLeft" => Some(XYPadAdjustment::Left),
                        "ArrowRight" => Some(XYPadAdjustment::Right),
                        "ArrowUp" => Some(XYPadAdjustment::Up),
                        "ArrowDown" => Some(XYPadAdjustment::Down),
                        _ => None,
                    })
                    .flatten();
                if let Some(adjustment) = xy_adjustment
                    && context.adjust_focused_xy_pad(document, adjustment)?
                {
                    return Ok(InputDisposition {
                        handled: true,
                        prevent_default: true,
                    });
                }
                #[cfg(feature = "graph-canvas")]
                let graph_adjustment = (!primary)
                    .then_some(match key.as_str() {
                        "ArrowLeft" => Some(GraphCanvasAdjustment::PanLeft),
                        "ArrowRight" => Some(GraphCanvasAdjustment::PanRight),
                        "ArrowUp" => Some(GraphCanvasAdjustment::PanUp),
                        "ArrowDown" => Some(GraphCanvasAdjustment::PanDown),
                        "Home" | "0" => Some(GraphCanvasAdjustment::Fit),
                        "+" | "=" => Some(GraphCanvasAdjustment::ZoomIn),
                        "-" => Some(GraphCanvasAdjustment::ZoomOut),
                        "Escape" => Some(GraphCanvasAdjustment::ClearSelection),
                        _ => None,
                    })
                    .flatten();
                #[cfg(feature = "graph-canvas")]
                if let Some(adjustment) = graph_adjustment
                    && optional_input!(
                        "graph-canvas",
                        context.adjust_focused_graph_canvas(document, adjustment),
                        Ok::<bool, FrameworkError>(false)
                    )?
                {
                    return Ok(InputDisposition {
                        handled: true,
                        prevent_default: true,
                    });
                }
                let split_direction = (!primary)
                    .then_some(match key.as_str() {
                        "ArrowLeft" | "ArrowUp" => Some(-1.0),
                        "ArrowRight" | "ArrowDown" => Some(1.0),
                        _ => None,
                    })
                    .flatten();
                if let Some(direction) = split_direction
                    && (context.adjust_focused_split(document, direction)?
                        || context.adjust_focused_dock_split(document, direction)?)
                {
                    return Ok(InputDisposition {
                        handled: true,
                        prevent_default: true,
                    });
                }
                if !primary {
                    // A numeric field's step and commit keys are routed with
                    // its other editing keys in `text_editor_key`; Escape is
                    // not an editing key and reverts the draft here.
                    if matches!(key.as_str(), "Escape")
                        && context.revert_focused_number_input(document)?
                    {
                        return Ok(InputDisposition {
                            handled: true,
                            prevent_default: true,
                        });
                    }
                    let palette_nav = match key.as_str() {
                        "ArrowUp" => Some(nana_ui_runtime::ActionPickerNavigation::Previous),
                        "ArrowDown" => Some(nana_ui_runtime::ActionPickerNavigation::Next),
                        "Home" => Some(nana_ui_runtime::ActionPickerNavigation::First),
                        "End" => Some(nana_ui_runtime::ActionPickerNavigation::Last),
                        "Enter" => Some(nana_ui_runtime::ActionPickerNavigation::Confirm),
                        "Escape" => Some(nana_ui_runtime::ActionPickerNavigation::Dismiss),
                        _ => None,
                    };
                    if let Some(navigation) = palette_nav
                        && context.navigate_focused_command_palette(document, navigation)?
                    {
                        return Ok(InputDisposition {
                            handled: true,
                            prevent_default: true,
                        });
                    }
                    let select_delta = match key.as_str() {
                        "ArrowUp" => Some(-1),
                        "ArrowDown" => Some(1),
                        _ => None,
                    };
                    if let Some(delta) = select_delta
                        && (context.adjust_focused_select(document, delta)?
                            || context.adjust_focused_dropdown(document, delta)?
                            || context.adjust_focused_search_dropdown(document, delta)?)
                    {
                        return Ok(InputDisposition {
                            handled: true,
                            prevent_default: true,
                        });
                    }
                    if matches!(key.as_str(), " " | "Space" | "Enter")
                        && (context.commit_focused_select(document)?
                            || context.commit_focused_dropdown(document)?)
                    {
                        return Ok(InputDisposition {
                            handled: true,
                            prevent_default: true,
                        });
                    }
                    if matches!(key.as_str(), "Enter")
                        && context.commit_focused_search_dropdown(document)?
                    {
                        return Ok(InputDisposition {
                            handled: true,
                            prevent_default: true,
                        });
                    }
                    let tree_nav = match key.as_str() {
                        "ArrowUp" => Some(nana_ui_runtime::TreeNavigation::Previous),
                        "ArrowDown" => Some(nana_ui_runtime::TreeNavigation::Next),
                        "Home" => Some(nana_ui_runtime::TreeNavigation::First),
                        "End" => Some(nana_ui_runtime::TreeNavigation::Last),
                        "ArrowLeft" => Some(nana_ui_runtime::TreeNavigation::Parent),
                        "ArrowRight" => Some(nana_ui_runtime::TreeNavigation::Child),
                        "Enter" => Some(nana_ui_runtime::TreeNavigation::Activate),
                        " " | "Space" => Some(nana_ui_runtime::TreeNavigation::Toggle),
                        _ => None,
                    };
                    if let Some(navigation) = tree_nav
                        && context.navigate_focused_tree(document, navigation)?
                    {
                        return Ok(InputDisposition {
                            handled: true,
                            prevent_default: true,
                        });
                    }
                }
                if !primary
                    && matches!(key.as_str(), " " | "Space" | "Enter")
                    && let Some(target) = context.world().focused(document)
                    && context.activate_node(target)?
                {
                    return Ok(InputDisposition {
                        handled: true,
                        prevent_default: true,
                    });
                }
                let navigation = match (key.as_str(), primary) {
                    ("ArrowUp", false) => Some(TableNavigation::PreviousRow),
                    ("ArrowDown", false) => Some(TableNavigation::NextRow),
                    ("ArrowLeft", false) => Some(TableNavigation::PreviousColumn),
                    ("ArrowRight", false) => Some(TableNavigation::NextColumn),
                    ("Home", false) => Some(TableNavigation::RowStart),
                    ("End", false) => Some(TableNavigation::RowEnd),
                    ("Home", true) => Some(TableNavigation::FirstRow),
                    ("End", true) => Some(TableNavigation::LastRow),
                    ("PageUp", false) => Some(TableNavigation::PageUp),
                    ("PageDown", false) => Some(TableNavigation::PageDown),
                    _ => None,
                };
                if let Some(navigation) = navigation {
                    context.navigate_focused_table(document, navigation, self.table_page_rows)?
                } else if !primary && !modifiers.alt {
                    match key.as_str() {
                        "Backspace" => context.delete_focused_text_backward(document)?,
                        _ => match typed_text(text.as_deref()) {
                            Some(text) => context.replace_focused_text(document, text)?,
                            None => false,
                        },
                    }
                } else {
                    false
                }
            }
            InputEvent::Keyboard {
                pressed,
                text,
                modifiers,
                ..
            } if *pressed && !modifiers.alt && !modifiers.control && !modifiers.meta => {
                match typed_text(text.as_deref()) {
                    Some(text) => context.replace_focused_text(document, text)?,
                    None => false,
                }
            }
            _ => false,
        };
        Ok(InputDisposition {
            handled,
            prevent_default: handled || keyboard_barrier,
        })
    }

    /// Apply Ctrl/Cmd + C / X / V / A to the focused Runtime editor.
    ///
    /// The Runtime owns what is selected and what an edit does; this adapter
    /// only moves text between that selection and the host pasteboard. A copy
    /// with nothing selected leaves the pasteboard alone rather than clearing
    /// it, and a paste with an empty pasteboard is not an edit.
    fn dispatch_clipboard_shortcut(
        &self,
        context: &mut AppContext,
        document: DocumentId,
        key: &str,
    ) -> Result<bool, FrameworkError> {
        if key.eq_ignore_ascii_case("a") {
            return context.select_all_focused_text(document);
        }
        if key.eq_ignore_ascii_case("c") {
            let text = context
                .focused_selected_text(document)
                .or_else(|| context.document_selected_text(document));
            return Ok(text.is_some_and(|text| !text.is_empty() && self.write_clipboard(&text)));
        }
        if key.eq_ignore_ascii_case("x") {
            let Some(text) = context.focused_selected_text(document) else {
                return Ok(false);
            };
            // Never delete text the pasteboard refused to take.
            if !self.write_clipboard(&text) {
                return Ok(false);
            }
            context.cut_focused_text(document)?;
            return Ok(true);
        }
        if key.eq_ignore_ascii_case("v") {
            let Some(text) = self.read_clipboard() else {
                return Ok(false);
            };
            return context.paste_focused_text(document, &text);
        }
        Ok(false)
    }

    /// Route platform IME into the focused Runtime editor.
    ///
    /// Retained TextInput/TextArea/SearchDropdown/CommandPalette state is the
    /// only editing authority. A
    /// focused editable field, or a blocking overlay, consumes the event so a
    /// second host IME path cannot also mutate it.
    ///
    /// Multi-cursor restriction: composition is anchored to the primary
    /// cursor only. While preedit is active the editor paints a single caret
    /// and, on commit, only the primary selection's text is replaced; the
    /// additional cursors survive through offset remapping.
    pub fn dispatch_ime(
        &self,
        context: &mut AppContext,
        document: DocumentId,
        event: &ImeEvent,
    ) -> Result<InputDisposition, FrameworkError> {
        let overlay_blocks = context.has_blocking_runtime_overlay(document);
        if !overlay_blocks && context.focused_terminal(document).is_some() {
            match event {
                ImeEvent::Preedit { text, .. } => {
                    context.set_terminal_preedit(document, text)?;
                }
                ImeEvent::Commit(text) => {
                    context.set_terminal_preedit(document, "")?;
                    context.terminal_input(document, text.as_bytes().to_vec())?;
                }
                ImeEvent::Disabled | ImeEvent::Cancelled => {
                    context.set_terminal_preedit(document, "")?;
                }
                ImeEvent::Enabled | ImeEvent::DeleteSurrounding { .. } => {}
            }
            return Ok(InputDisposition {
                handled: true,
                prevent_default: true,
            });
        }

        let owns_ime = context
            .focused_text_input(document)
            .is_some_and(|(target, _)| {
                context
                    .world()
                    .accessibility(target)
                    .is_some_and(|state| state.editable)
            });
        let handled = match event {
            ImeEvent::Enabled => false,
            ImeEvent::Disabled => {
                let leftover = context
                    .world()
                    .focused_text_input(document)
                    .and_then(|(id, _)| context.world().ime(id).map(|ime| ime.text.to_owned()))
                    .filter(|text| !text.is_empty());
                match leftover {
                    Some(text) => context.commit_ime(document, &text)?,
                    None => context.clear_ime(document)?,
                }
            }
            ImeEvent::Cancelled => context.clear_ime(document)?,
            ImeEvent::Preedit { text, selection } => {
                context.set_ime_preedit(document, text.clone(), *selection)?
            }
            ImeEvent::Commit(text) => context.commit_ime(document, text)?,
            ImeEvent::DeleteSurrounding {
                before_bytes,
                after_bytes,
            } => context.delete_ime_surrounding(document, *before_bytes, *after_bytes)?,
        };
        Ok(InputDisposition {
            handled,
            prevent_default: handled || owns_ime || overlay_blocks,
        })
    }
}

/// 补全弹层在激活期间消费的编辑键。
enum CompletionKey {
    Up,
    Down,
    Accept,
}

impl RuntimeInputAdapter {
    /// Keyboard editing for the focused plain text editor.
    ///
    /// Returns `false` when no plain editor is focused or the key is not an
    /// editing key, so composite-surface navigation and generic activation
    /// keep working unchanged.
    fn text_editor_key(
        context: &mut AppContext,
        document: DocumentId,
        key: &str,
        text: Option<&str>,
        modifiers: nana_ui_platform::InputModifiers,
        mut shaper: Option<&mut dyn TextShaper>,
    ) -> Result<bool, FrameworkError> {
        let Some(focused) = context.focused_text_editor(document) else {
            // An IME composition hides the editor, but its navigation keys
            // still belong to it: the composition owns them, and they must
            // not reach an enclosing table, tree or select and carry focus
            // out of the field.
            return Ok(caret_intent(key, modifiers).is_some()
                && context.focused_text_editor_composing(document));
        };
        // A numeric field steps on plain ArrowUp/ArrowDown and commits its
        // draft on Enter. Shift+ArrowUp/Down select like any single-line
        // field. The arrows are the field's even when nothing moves (a bound,
        // read-only), so they never fall through to an enclosing table or
        // tree and carry focus out of the field. An Enter that commits
        // nothing falls through, so a dialog or form can still confirm.
        // Alt+ArrowUp/Down move the caret like any single-line field's; Enter
        // commits whatever Shift or Alt accompany it.
        if focused.is_numeric() && !modifiers.control && !modifiers.meta {
            match key {
                "ArrowUp" | "ArrowDown" if !modifiers.shift && !modifiers.alt => {
                    let steps = if key == "ArrowUp" { 1 } else { -1 };
                    context.step_focused_number_input(document, steps)?;
                    return Ok(true);
                }
                "Enter" => return context.commit_focused_number_input(document),
                _ => {}
            }
        }
        // Arrow keys in the editor's line space (#59): a vertical editor's
        // Up/Down walk its column and Left/Right cross columns, with every
        // modifier below carried along by translating the key itself.
        let key = context.focused_text_line_space_key(document, key);
        if key == "Tab"
            && focused.multiline
            && !modifiers.control
            && !modifiers.meta
            && !modifiers.alt
            && context.advance_focused_text_snippet(document, modifiers.shift)?
        {
            return Ok(true);
        } // 补全弹层激活时，无修饰的 Up/Down/Enter/Tab 由弹层消费：Up/Down
        // 移动候选选中项（编辑器选区不动），Enter/Tab 接受选中项。其余键
        // 穿透正常编辑（打字触发宿主重喂过滤列表）；任何修饰键组合
        // （Cmd+D、Alt+Up、Shift+Up 等）一律穿透。
        if !modifiers.control && !modifiers.meta && !modifiers.alt {
            let completion_key = match key {
                "ArrowUp" if !modifiers.shift => Some(CompletionKey::Up),
                "ArrowDown" if !modifiers.shift => Some(CompletionKey::Down),
                "Enter" if !modifiers.shift => Some(CompletionKey::Accept),
                "Tab" if !modifiers.shift => Some(CompletionKey::Accept),
                _ => None,
            };
            if let Some(completion_key) = completion_key
                && context.focused_text_completion_active(document)
            {
                match completion_key {
                    CompletionKey::Up => {
                        context.move_focused_text_completion(document, false)?;
                    }
                    CompletionKey::Down => {
                        context.move_focused_text_completion(document, true)?;
                    }
                    CompletionKey::Accept => {
                        context.accept_focused_text_completion(document, None)?;
                    }
                }
                // 弹层激活期间整键消费（边界上导航无可做也不移动选区）。
                return Ok(true);
            }
        }
        let control = modifiers.control;
        let meta = modifiers.meta;
        // Alt+Cmd/Ctrl+Up/Down adds cursors above/below the selection(s)
        // (Zed-style multi-cursor). Multiline editors own the gesture even
        // when every target already holds a cursor; single-line fields
        // reject multi-cursor entirely and keep plain movement.
        if modifiers.alt
            && (control || meta)
            && focused.multiline
            && matches!(key, "ArrowUp" | "ArrowDown")
        {
            context.add_focused_text_cursor(
                document,
                key == "ArrowUp",
                reborrow_text_shaper(&mut shaper),
            )?;
            return Ok(true);
        }
        // Alt+Up/Down moves the caret's line block; Alt+Shift+Up/Down
        // duplicates it. Multiline editors own the gesture even at the
        // document edge; single-line fields keep plain caret movement.
        if modifiers.alt
            && !control
            && !meta
            && focused.multiline
            && matches!(key, "ArrowUp" | "ArrowDown")
        {
            let direction = if key == "ArrowUp" {
                TextLineDirection::Up
            } else {
                TextLineDirection::Down
            };
            if modifiers.shift {
                context.duplicate_focused_text_lines(document)?;
            } else {
                context.move_focused_text_lines(document, direction)?;
            }
            return Ok(true);
        }
        if let Some(intent) = caret_intent(key, modifiers) {
            return context.move_focused_text_caret(document, intent, modifiers.shift, shaper);
        }
        let delete = match (key, control || meta, modifiers.alt) {
            ("Backspace", false, false) => Some(TextDeleteKind::Backward),
            ("Backspace", _, true) => Some(TextDeleteKind::WordBackward),
            ("Backspace", true, false) => Some(TextDeleteKind::LineStart),
            ("Delete", false, false) => Some(TextDeleteKind::Forward),
            ("Delete", _, true) => Some(TextDeleteKind::WordForward),
            ("Delete", true, false) => Some(TextDeleteKind::LineEnd),
            _ => None,
        };
        if let Some(kind) = delete {
            return context.delete_focused_text(document, kind);
        }
        if control || meta {
            // Cmd/Ctrl+D selects the next occurrence of the primary
            // selection's word (Zed-style multi-cursor). Multiline only.
            // Cmd/Ctrl+Shift+D is deliberately unbound for now; hosts can
            // call `select_focused_text_occurrence(document, true)` for the
            // reverse direction.
            if key.eq_ignore_ascii_case("d") && focused.multiline && !modifiers.shift {
                return context.select_focused_text_occurrence(document, false);
            }
            // Comment toggle is the only code-editing modified key.
            if key == "/" && focused.code_editing.is_some() {
                return context.code_edit_toggle_comment(document);
            }
            // Cmd/Ctrl+Shift+K deletes the caret line.
            if modifiers.shift && key.eq_ignore_ascii_case("k") {
                return context.delete_focused_text_lines(document);
            }
            // Ctrl/Cmd+J joins the touched selection lines.
            if !modifiers.shift && key.eq_ignore_ascii_case("j") {
                return context.join_focused_text_lines(document);
            }
            // Ctrl/Cmd+Shift+U uppercases, Ctrl/Cmd+U lowercases.
            if key.eq_ignore_ascii_case("u") {
                return context.transform_focused_text_case(document, modifiers.shift);
            }
            return Ok(false);
        }
        if key == "Enter" {
            if focused.multiline {
                return context.insert_focused_text_newline(document);
            }
            // Single-line fields submit without inserting a newline. IME
            // confirmation remains exclusively owned by the composition path.
            context.submit_focused_text_input(document)?;
            return Ok(true);
        }
        if key == "Tab" && !meta {
            // snippet 会话内 Tab 跳位优先于缩进；无会话时 `Ok(false)`，
            // 代码编辑器的缩进行为接手。
            if focused.multiline
                && context.advance_focused_text_snippet(document, modifiers.shift)?
            {
                return Ok(true);
            }
            if focused.code_editing.is_some() {
                return context.code_edit_indent(document, modifiers.shift);
            }
            return Ok(false);
        }
        let Some(text) = typed_text(text).filter(|_| key != "Escape") else {
            return Ok(false);
        };
        let mut typed = text.chars();
        if let (Some(single), None) = (typed.next(), typed.next())
            && focused.code_editing.is_some()
            && context.code_edit_typed(document, single)?
        {
            return Ok(true);
        }
        context.replace_focused_text(document, text)
    }
}

/// The caret move an editor's navigation key asks for, in line space.
/// `None` for keys that are not caret navigation.
fn caret_intent(key: &str, modifiers: nana_ui_platform::InputModifiers) -> Option<TextCaretIntent> {
    let (control, meta) = (modifiers.control, modifiers.meta);
    let word_modifier = control || modifiers.alt;
    match key {
        "ArrowLeft" => Some(match (meta, word_modifier) {
            (true, _) => TextCaretIntent::LineStart,
            (_, true) => TextCaretIntent::WordLeft,
            (false, false) => TextCaretIntent::Left,
        }),
        "ArrowRight" => Some(match (meta, word_modifier) {
            (true, _) => TextCaretIntent::LineEnd,
            (_, true) => TextCaretIntent::WordRight,
            _ => TextCaretIntent::Right,
        }),
        "ArrowUp" => Some(if meta {
            TextCaretIntent::DocStart
        } else {
            TextCaretIntent::Up
        }),
        "ArrowDown" => Some(if meta {
            TextCaretIntent::DocEnd
        } else {
            TextCaretIntent::Down
        }),
        "Home" => Some(if control || meta {
            TextCaretIntent::DocStart
        } else {
            TextCaretIntent::LineStart
        }),
        "End" => Some(if control || meta {
            TextCaretIntent::DocEnd
        } else {
            TextCaretIntent::LineEnd
        }),
        "PageUp" if !control && !meta && !modifiers.alt => Some(TextCaretIntent::PageUp),
        "PageDown" if !control && !meta && !modifiers.alt => Some(TextCaretIntent::PageDown),
        _ => None,
    }
}

/// The text a key carries, or `None` when it carries none. Which characters
/// may type is the runtime's call: its typing path refuses the control
/// characters command keys carry.
fn typed_text(text: Option<&str>) -> Option<&str> {
    text.filter(|text| !text.is_empty())
}

/// Reborrow the per-dispatch shaper so sequential uses never alias.
fn reborrow_text_shaper<'s>(
    shaper: &'s mut Option<&mut dyn TextShaper>,
) -> Option<&'s mut dyn TextShaper> {
    match shaper.as_mut() {
        Some(shaper) => Some(&mut **shaper),
        None => None,
    }
}

fn nearest_focusable(context: &AppContext, mut target: StableNodeId) -> Option<StableNodeId> {
    loop {
        if context
            .world()
            .interaction(target)
            .is_some_and(|interaction| interaction.focusable)
        {
            return Some(target);
        }
        target = context.world().node(target).and_then(|node| node.parent)?;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nana_ui_core::{LayoutStyle, OverflowSpec};
    use nana_ui_platform::{
        ImeEvent, InputModifiers, MemoryClipboard, PointerType, shared_clipboard,
    };
    use nana_ui_runtime::{
        ActionMenu, ActionMenuItem, Activate, Button, Card, ComponentGeometry, Dialog, Dock,
        DockAxis, DockNode, Entity, LayoutBox, MeasureTextShaper, ModalSlots, MutationQueue,
        NodeKind, NodeStyle, OverlayHost, OverlayHostState, RangeField, ScrollAxes, ScrollMetrics,
        ScrollView, SegmentedControl, SegmentedOption, SegmentedSelectionRequested, Table,
        TableCell, TableRow, Text, TextArea, TextChanged, TextFindScope, TextInput,
        TextSearchOptions, TextSelection, UserSelectSpec,
    };
    #[cfg(feature = "calendar")]
    use nana_ui_runtime::{CalendarHeatmap, CalendarHeatmapDatum};
    #[cfg(feature = "graph-canvas")]
    use nana_ui_runtime::{
        GraphMinimap, GraphMinimapEvent, GraphModel, GraphNode, GraphPoint, GraphSize,
        GraphViewport,
    };
    use std::sync::{Arc, Mutex};

    fn wheel(x: f32, y: f32, delta_y: f32) -> InputEvent {
        InputEvent::Wheel {
            x,
            y,
            delta_x: 0.0,
            delta_y,
            line_delta: true,
            modifiers: InputModifiers::default(),
        }
    }

    fn focused_untyped_text_input(
        context: &mut AppContext,
        value: &str,
    ) -> (DocumentId, nana_ui_runtime::StableNodeId) {
        let document = DocumentId::new(1).unwrap();
        let id = nana_ui_runtime::StableNodeId::new(1).unwrap();
        let mut create = MutationQueue::new();
        create.create(
            id,
            document,
            nana_ui_runtime::NodeKind::Element {
                tag: "input".into(),
            },
        );
        create.set_interaction(
            id,
            nana_ui_runtime::InteractionState {
                pointer_events: true,
                focusable: true,
            },
        );
        create.set_text_input(id, Some(nana_ui_runtime::TextInputState::new(value)));
        create.set_accessibility(
            id,
            nana_ui_runtime::AccessibilityState {
                role: nana_ui_runtime::AccessibilityRole::TextInput,
                editable: true,
                ..nana_ui_runtime::AccessibilityState::default()
            },
        );
        create.request_focus(document, Some(id));
        context.commit_mutations(create).unwrap();
        (document, id)
    }

    fn pointer(phase: PointerPhase, x: f32, y: f32) -> InputEvent {
        pointer_with(phase, x, y, false)
    }

    fn pointer_with(phase: PointerPhase, x: f32, y: f32, activation_click: bool) -> InputEvent {
        InputEvent::Pointer {
            phase,
            pointer_id: 1,
            pointer_type: PointerType::Mouse,
            x,
            y,
            screen_x: x,
            screen_y: y,
            button: 0,
            buttons: u16::from(phase == PointerPhase::Down),
            pressure: 0.0,
            tangential_pressure: 0.0,
            tilt_x: 0,
            tilt_y: 0,
            twist: 0,
            is_primary: true,
            activation_click,
            modifiers: InputModifiers::default(),
        }
    }

    #[test]
    fn pointer_release_activates_the_retained_hit_target() {
        let mut context = AppContext::new();
        let document = DocumentId::new(1).unwrap();
        let button = context
            .create_component(document, Button::new("Build"))
            .unwrap();
        context
            .on(button, |button, _event: &Activate, _cx| {
                button.label = "Running".into();
            })
            .unwrap();
        let mut layout = MutationQueue::new();
        layout.write_layout(
            button.stable_id(),
            LayoutBox {
                x: 10.0,
                y: 20.0,
                width: 120.0,
                height: 32.0,
            },
        );
        context.commit_mutations(layout).unwrap();
        context.take_system_work();
        context.rebuild_hit_test(document);

        let mut adapter = RuntimeInputAdapter::default();
        assert!(
            adapter
                .dispatch(
                    &mut context,
                    document,
                    &pointer(PointerPhase::Down, 30.0, 30.0)
                )
                .unwrap()
                .prevent_default
        );
        assert!(
            adapter
                .dispatch(
                    &mut context,
                    document,
                    &pointer(PointerPhase::Up, 30.0, 30.0)
                )
                .unwrap()
                .prevent_default
        );
        assert_eq!(context.world().text(button.stable_id()), Some("Running"));
    }

    #[test]
    fn text_area_resize_routes_grip_drag_cancel_and_park_without_editing_text() {
        let mut context = AppContext::new();
        let document = DocumentId::new(1).unwrap();
        let area = context
            .create_component(
                document,
                TextArea::new("retained text")
                    .height(100.0)
                    .resize_vertical(true),
            )
            .unwrap();
        let viewport = nana_ui_runtime::LayoutViewport::new(320.0, 500.0);
        context.resolve_styles(&[area.stable_id()]).unwrap();
        context
            .shape_text(&[area.stable_id()], &mut MeasureTextShaper)
            .unwrap();
        context.layout_document(document, viewport).unwrap();
        context.rebuild_hit_test(document);
        let Some(ComponentGeometry::TextInput {
            resize_grip: Some(grip),
            ..
        }) = context.world().component_geometry(area.stable_id())
        else {
            panic!("resize grip must be projected")
        };
        let x = grip.x + grip.width / 2.0;
        let y = grip.y + grip.height / 2.0;
        let initial = context.world().layout_box(area.stable_id()).unwrap().height;
        let mut adapter = RuntimeInputAdapter::default();
        assert!(
            adapter
                .dispatch(&mut context, document, &pointer(PointerPhase::Down, x, y))
                .unwrap()
                .prevent_default
        );
        assert_eq!(
            context.world().pointer_capture(document, 1),
            Some(area.stable_id())
        );
        adapter
            .dispatch(
                &mut context,
                document,
                &pointer(PointerPhase::Move, x, y + 45.0),
            )
            .unwrap();
        context.layout_document(document, viewport).unwrap();
        assert!(
            (context.world().layout_box(area.stable_id()).unwrap().height - initial - 45.0).abs()
                < 0.01
        );
        adapter
            .dispatch(
                &mut context,
                document,
                &pointer(PointerPhase::Cancel, x, y + 45.0),
            )
            .unwrap();
        context.layout_document(document, viewport).unwrap();
        context.rebuild_hit_test(document);
        assert_eq!(
            context.world().layout_box(area.stable_id()).unwrap().height,
            initial
        );
        assert_eq!(context.world().pointer_capture(document, 1), None);
        context.resolve_styles(&[area.stable_id()]).unwrap();
        adapter
            .dispatch(&mut context, document, &pointer(PointerPhase::Down, x, y))
            .unwrap();
        context
            .update_text_area_resize(document, 1, x, y + 30.0)
            .unwrap();
        let other = context
            .create_component(document, Button::new("other capture"))
            .unwrap();
        let mut steal = MutationQueue::new();
        steal.capture_pointer(1, other.stable_id());
        context.commit_mutations(steal).unwrap();
        assert!(context.end_text_area_resize(document, 1, false).unwrap());
        assert_eq!(
            context.world().pointer_capture(document, 1),
            Some(other.stable_id())
        );
        context.remove_view(other).unwrap();
        context.resolve_styles(&[area.stable_id()]).unwrap();
        context.layout_document(document, viewport).unwrap();
        context.rebuild_hit_test(document);
        assert_eq!(
            context.world().layout_box(area.stable_id()).unwrap().height,
            initial
        );
        adapter
            .dispatch(&mut context, document, &pointer(PointerPhase::Down, x, y))
            .unwrap();
        let mut queue = MutationQueue::new();
        queue.park_subtree(area.stable_id());
        context.commit_mutations(queue).unwrap();
        assert!(
            !context
                .update_text_area_resize(document, 1, x, y + 70.0)
                .unwrap()
        );
        assert_eq!(context.world().pointer_capture(document, 1), None);
        assert_eq!(
            context.read(area, |area| area.state.value.clone()).unwrap(),
            "retained text"
        );
    }

    #[test]
    fn macos_activation_click_does_not_activate_the_hit_target() {
        let mut context = AppContext::new();
        let document = DocumentId::new(1).unwrap();
        let button = context
            .create_component(document, Button::new("Build"))
            .unwrap();
        context
            .on(button, |button, _event: &Activate, _cx| {
                button.label = "Running".into();
            })
            .unwrap();
        let mut layout = MutationQueue::new();
        layout.write_layout(
            button.stable_id(),
            LayoutBox {
                x: 10.0,
                y: 20.0,
                width: 120.0,
                height: 32.0,
            },
        );
        context.commit_mutations(layout).unwrap();
        context.take_system_work();
        context.rebuild_hit_test(document);

        let mut adapter = RuntimeInputAdapter::default();
        assert!(
            adapter
                .dispatch(
                    &mut context,
                    document,
                    &pointer_with(PointerPhase::Down, 30.0, 30.0, true)
                )
                .unwrap()
                .prevent_default
        );
        let _ = adapter.dispatch(
            &mut context,
            document,
            &pointer_with(PointerPhase::Up, 30.0, 30.0, true),
        );
        assert_eq!(context.world().text(button.stable_id()), Some("Build"));
    }

    #[cfg(feature = "rich-text")]
    #[test]
    fn markdown_link_pointer_uses_the_painted_padded_content_and_cancels_stolen_capture() {
        use nana_ui_runtime::{MarkdownDrawingCommand, NativeMarkdown, RichTextEvent};
        let mut context = AppContext::new();
        let document = DocumentId::new(1).unwrap();
        let mut markdown = NativeMarkdown::from_source("[Open](https://example.com)");
        let layout = Arc::make_mut(&mut markdown.style.layout);
        layout.padding = Some(nana_ui_core::LengthSpec::Px(28.0));
        layout.border_width = Some(3.0);
        let markdown = context.create_component(document, markdown).unwrap();
        let events = Arc::new(Mutex::new(Vec::new()));
        let sink = Arc::clone(&events);
        context
            .on(markdown, move |_, event: &RichTextEvent, _| {
                sink.lock().unwrap().push(event.clone())
            })
            .unwrap();
        context.resolve_styles(&[markdown.stable_id()]).unwrap();
        context
            .layout_document(document, nana_ui_runtime::LayoutViewport::new(300.0, 160.0))
            .unwrap();
        context.rebuild_hit_test(document);
        let Some(ComponentGeometry::NativeMarkdown { drawing, .. }) =
            context.world().component_geometry(markdown.stable_id())
        else {
            panic!("markdown geometry")
        };
        let bounds = drawing
            .commands
            .iter()
            .find_map(|command| match command {
                MarkdownDrawingCommand::Text { bounds, .. } => Some(*bounds),
                _ => None,
            })
            .expect("painted link text");
        let x = bounds.x + 1.0;
        let y = bounds.y + bounds.height / 2.0;
        assert!(bounds.x >= 31.0);
        let mut adapter = RuntimeInputAdapter::default();
        adapter
            .dispatch(&mut context, document, &pointer(PointerPhase::Down, x, y))
            .unwrap();
        adapter
            .dispatch(&mut context, document, &pointer(PointerPhase::Up, x, y))
            .unwrap();
        assert!(events.lock().unwrap().iter().any(|event| matches!(event, RichTextEvent::LinkActivated(url) if url.as_ref() == "https://example.com")));
        events.lock().unwrap().clear();
        adapter
            .dispatch(&mut context, document, &pointer(PointerPhase::Down, x, y))
            .unwrap();
        let other = context
            .create_component(document, Button::new("new owner"))
            .unwrap();
        let mut queue = MutationQueue::new();
        queue.capture_pointer(1, other.stable_id());
        context.commit_mutations(queue).unwrap();
        assert!(
            context
                .end_rich_text_pointer(document, 1, x, y, false)
                .unwrap()
        );
        assert_eq!(
            context.world().pointer_capture(document, 1),
            Some(other.stable_id())
        );
        assert!(events.lock().unwrap().is_empty());
    }

    #[test]
    fn a_right_button_press_dispatches_a_secondary_press_without_activating() {
        use nana_ui_runtime::SecondaryPress;
        use std::sync::{Arc, Mutex};

        let mut context = AppContext::new();
        let document = DocumentId::new(1).unwrap();
        let button = context
            .create_component(document, Button::new("Build"))
            .unwrap();
        let presses = Arc::new(Mutex::new(Vec::new()));
        let observed = Arc::clone(&presses);
        context
            .on(button, move |_button, press: &SecondaryPress, _cx| {
                observed.lock().unwrap().push(*press);
            })
            .unwrap();
        context
            .on(button, |button, _event: &Activate, _cx| {
                button.label = "Running".into();
            })
            .unwrap();
        let mut layout = MutationQueue::new();
        layout.write_layout(
            button.stable_id(),
            LayoutBox {
                x: 10.0,
                y: 20.0,
                width: 120.0,
                height: 32.0,
            },
        );
        context.commit_mutations(layout).unwrap();
        context.take_system_work();
        context.rebuild_hit_test(document);

        let mut adapter = RuntimeInputAdapter::default();
        let mut secondary = pointer(PointerPhase::Down, 30.0, 30.0);
        if let InputEvent::Pointer { button, .. } = &mut secondary {
            *button = 2;
        }
        assert!(
            adapter
                .dispatch(&mut context, document, &secondary)
                .unwrap()
                .prevent_default
        );
        let press = *presses
            .lock()
            .unwrap()
            .first()
            .expect("one secondary press");
        assert_eq!(press.target, button.stable_id());
        assert_eq!((press.x, press.y), (30.0, 30.0));

        // The release must not activate: no press was recorded for button 2.
        let mut release = pointer(PointerPhase::Up, 30.0, 30.0);
        if let InputEvent::Pointer { button, .. } = &mut release {
            *button = 2;
        }
        adapter.dispatch(&mut context, document, &release).unwrap();
        assert_eq!(context.world().text(button.stable_id()), Some("Build"));
    }

    #[test]
    #[cfg(feature = "graph-canvas")]
    fn pointer_drag_on_a_graph_minimap_requests_viewport_navigation() {
        let mut context = AppContext::new();
        let document = DocumentId::new(1).unwrap();
        let model = GraphModel::new(
            vec![GraphNode::new(
                "node",
                "Node",
                GraphPoint::ZERO,
                GraphSize::new(200.0, 100.0),
            )],
            Vec::new(),
        )
        .expect("valid graph");
        let minimap = context
            .create_component(
                document,
                GraphMinimap::new(model).canvas_size(GraphSize::new(400.0, 200.0)),
            )
            .unwrap();
        let events = Arc::new(Mutex::new(Vec::new()));
        let observed = Arc::clone(&events);
        context
            .on(minimap, move |_minimap, event: &GraphMinimapEvent, _cx| {
                observed.lock().unwrap().push(event.clone());
            })
            .unwrap();
        let mut layout = MutationQueue::new();
        layout.write_layout(
            minimap.stable_id(),
            LayoutBox {
                x: 0.0,
                y: 0.0,
                width: 100.0,
                height: 50.0,
            },
        );
        context.commit_mutations(layout).unwrap();
        context.take_system_work();
        context.rebuild_hit_test(document);

        let mut adapter = RuntimeInputAdapter::default();
        for (phase, x, y) in [
            (PointerPhase::Down, 50.0, 25.0),
            (PointerPhase::Move, 60.0, 30.0),
            (PointerPhase::Up, 60.0, 30.0),
        ] {
            assert!(
                adapter
                    .dispatch(&mut context, document, &pointer(phase, x, y))
                    .unwrap()
                    .prevent_default
            );
        }
        assert_eq!(
            *events.lock().unwrap(),
            [
                GraphMinimapEvent::ViewportRequested(GraphViewport::new(
                    GraphPoint::new(100.0, 50.0),
                    1.0
                )),
                GraphMinimapEvent::ViewportRequested(GraphViewport::new(
                    GraphPoint::new(80.0, 40.0),
                    1.0
                )),
            ]
        );
        assert!(context.world().pointer_capture(document, 1).is_none());
    }

    #[test]
    fn pointer_down_moves_focus_to_the_hit_text_input() {
        let mut context = AppContext::new();
        let document = DocumentId::new(1).unwrap();
        let button = context
            .create_component(document, Button::new("Other"))
            .unwrap();
        let input = context
            .create_component(document, TextInput::new("NanaUI"))
            .unwrap();
        assert!(context.focus_node(document, button.stable_id()).unwrap());
        let mut layout = MutationQueue::new();
        layout.write_layout(
            button.stable_id(),
            LayoutBox {
                x: 0.0,
                y: 0.0,
                width: 120.0,
                height: 32.0,
            },
        );
        layout.write_layout(
            input.stable_id(),
            LayoutBox {
                x: 0.0,
                y: 40.0,
                width: 160.0,
                height: 32.0,
            },
        );
        context.commit_mutations(layout).unwrap();
        context.take_system_work();
        context.rebuild_hit_test(document);

        let mut adapter = RuntimeInputAdapter::default();
        assert!(
            adapter
                .dispatch(
                    &mut context,
                    document,
                    &pointer(PointerPhase::Down, 24.0, 52.0)
                )
                .unwrap()
                .prevent_default
        );
        assert_eq!(context.world().focused(document), Some(input.stable_id()));
    }

    #[test]
    fn focused_textarea_caret_uses_text_color_and_clears_on_outside_press() {
        let mut context = AppContext::new();
        let document = DocumentId::new(1).unwrap();
        let editor = context
            .create_component(document, TextArea::new("draft"))
            .unwrap();
        let surface = context.create_component(document, Card::new()).unwrap();
        let mut layout = MutationQueue::new();
        layout.write_layout(
            editor.stable_id(),
            LayoutBox {
                x: 0.0,
                y: 0.0,
                width: 200.0,
                height: 72.0,
            },
        );
        layout.write_layout(
            surface.stable_id(),
            LayoutBox {
                x: 0.0,
                y: 90.0,
                width: 200.0,
                height: 80.0,
            },
        );
        context.commit_mutations(layout).unwrap();
        assert!(context.focus_node(document, editor.stable_id()).unwrap());
        let work = context.take_system_work();
        context
            .compat_world_mut()
            .resolve_styles(&work.style)
            .unwrap();
        context
            .compat_world_mut()
            .shape_text(&work.text, &mut MeasureTextShaper)
            .unwrap();
        context.rebuild_hit_test(document);

        let Some(ComponentGeometry::TextInput {
            caret, caret_color, ..
        }) = context.world().component_geometry(editor.stable_id())
        else {
            panic!("expected text input geometry");
        };
        let palette = nana_ui_core::SemanticPalette::dark();
        assert!(caret.is_some());
        assert_eq!(caret_color, palette.text.as_rgba_array());
        assert_ne!(caret_color, palette.accent.as_rgba_array());

        RuntimeInputAdapter::default()
            .dispatch(
                &mut context,
                document,
                &pointer(PointerPhase::Down, 24.0, 120.0),
            )
            .unwrap();
        assert_eq!(context.world().focused(document), None);
        assert!(matches!(
            context.world().component_geometry(editor.stable_id()),
            Some(ComponentGeometry::TextInput { caret: None, .. })
        ));

        assert!(context.focus_node(document, editor.stable_id()).unwrap());
        RuntimeInputAdapter::default()
            .dispatch(
                &mut context,
                document,
                &pointer(PointerPhase::Down, 400.0, 400.0),
            )
            .unwrap();
        assert_eq!(context.world().focused(document), None);
    }

    #[test]
    fn segmented_pointer_lease_consumes_release_and_only_requests_on_inside_up() {
        let mut context = AppContext::new();
        let document = DocumentId::new(1).unwrap();
        let control = context
            .create_component(document, SegmentedControl::new())
            .unwrap();
        let first = context
            .create_detached_component(document, SegmentedOption::new("Code"))
            .unwrap();
        let second = context
            .create_detached_component(document, SegmentedOption::new("Preview"))
            .unwrap();
        context
            .set_segmented_options(control, vec![first, second], Some(first))
            .unwrap();
        let requests = Arc::new(Mutex::new(0));
        let observed = Arc::clone(&requests);
        context
            .on(
                control,
                move |_control, _event: &SegmentedSelectionRequested, _cx| {
                    *observed.lock().unwrap() += 1;
                },
            )
            .unwrap();
        let mut layout = MutationQueue::new();
        layout.write_layout(
            control.stable_id(),
            LayoutBox {
                x: 0.0,
                y: 0.0,
                width: 180.0,
                height: 32.0,
            },
        );
        layout.write_layout(
            first.stable_id(),
            LayoutBox {
                x: 4.0,
                y: 3.0,
                width: 70.0,
                height: 26.0,
            },
        );
        layout.write_layout(
            second.stable_id(),
            LayoutBox {
                x: 76.0,
                y: 3.0,
                width: 70.0,
                height: 26.0,
            },
        );
        context.commit_mutations(layout).unwrap();
        context.rebuild_hit_test(document);
        let mut adapter = RuntimeInputAdapter::default();

        assert!(
            adapter
                .dispatch(
                    &mut context,
                    document,
                    &pointer(PointerPhase::Down, 90.0, 12.0)
                )
                .unwrap()
                .prevent_default
        );
        assert_eq!(context.world().focused(document), Some(second.stable_id()));
        assert_eq!(
            context
                .read(control, SegmentedControl::focus_target)
                .unwrap(),
            Some(second.stable_id())
        );
        assert!(
            adapter
                .dispatch(
                    &mut context,
                    document,
                    &pointer(PointerPhase::Cancel, 90.0, 12.0)
                )
                .unwrap()
                .prevent_default
        );
        assert_eq!(*requests.lock().unwrap(), 0);

        assert!(
            adapter
                .dispatch(
                    &mut context,
                    document,
                    &pointer(PointerPhase::Down, 20.0, 12.0)
                )
                .unwrap()
                .prevent_default
        );
        assert!(
            adapter
                .dispatch(
                    &mut context,
                    document,
                    &pointer(PointerPhase::Up, 140.0, 12.0)
                )
                .unwrap()
                .prevent_default
        );
        assert_eq!(*requests.lock().unwrap(), 0);
        assert!(
            adapter
                .dispatch(
                    &mut context,
                    document,
                    &pointer(PointerPhase::Down, 20.0, 12.0)
                )
                .unwrap()
                .prevent_default
        );
        assert!(
            adapter
                .dispatch(
                    &mut context,
                    document,
                    &pointer(PointerPhase::Up, 20.0, 12.0)
                )
                .unwrap()
                .prevent_default
        );
        assert_eq!(*requests.lock().unwrap(), 1);
        assert!(context.read(first, SegmentedOption::selected).unwrap());
        assert!(
            adapter
                .dispatch(
                    &mut context,
                    document,
                    &pointer(PointerPhase::Down, 20.0, 12.0)
                )
                .unwrap()
                .prevent_default
        );
        assert!(
            adapter
                .dispatch(
                    &mut context,
                    document,
                    &pointer(PointerPhase::Cancel, 20.0, 12.0)
                )
                .unwrap()
                .prevent_default
        );
        assert_eq!(*requests.lock().unwrap(), 1);
    }

    #[test]
    fn document_tab_order_uses_one_roving_entry_and_wraps_both_directions() {
        let mut context = AppContext::new();
        let document = DocumentId::new(1).unwrap();
        let before = context
            .create_component(document, Button::new("Before"))
            .unwrap();
        let control = context
            .create_component(document, SegmentedControl::new())
            .unwrap();
        let first = context
            .create_detached_component(document, SegmentedOption::new("Code"))
            .unwrap();
        let second = context
            .create_detached_component(document, SegmentedOption::new("Preview"))
            .unwrap();
        let after = context
            .create_component(document, Button::new("After"))
            .unwrap();
        context
            .set_segmented_options(control, vec![first, second], Some(first))
            .unwrap();
        context.focus_node(document, before.stable_id()).unwrap();
        let tab = |shift| InputEvent::Keyboard {
            pressed: true,
            key: "Tab".into(),
            text: None,
            code: "Tab".into(),
            repeat: false,
            modifiers: InputModifiers {
                shift,
                ..InputModifiers::default()
            },
        };
        let mut adapter = RuntimeInputAdapter::default();
        for expected in [first.stable_id(), after.stable_id(), before.stable_id()] {
            assert!(
                adapter
                    .dispatch(&mut context, document, &tab(false))
                    .unwrap()
                    .prevent_default
            );
            assert_eq!(context.world().focused(document), Some(expected));
        }
        assert!(
            adapter
                .dispatch(&mut context, document, &tab(true))
                .unwrap()
                .prevent_default
        );
        assert_eq!(context.world().focused(document), Some(after.stable_id()));
        assert!(context.focus_node(document, second.stable_id()).unwrap());
    }

    #[test]
    fn focused_runtime_text_uses_keyboard_and_ime_state() {
        let mut context = AppContext::new();
        let document = DocumentId::new(1).unwrap();
        let input = context
            .create_component(document, TextInput::new("Nana"))
            .unwrap();
        assert!(context.focus_node(document, input.stable_id()).unwrap());
        let key = |key: &str| InputEvent::Keyboard {
            pressed: true,
            key: key.into(),
            text: (key.chars().count() == 1).then(|| key.into()),
            code: key.into(),
            repeat: false,
            modifiers: InputModifiers::default(),
        };

        let mut adapter = RuntimeInputAdapter::default();
        assert!(
            adapter
                .dispatch(&mut context, document, &key("U"))
                .unwrap()
                .prevent_default
        );
        assert_eq!(context.world().text(input.stable_id()), Some("NanaU"));
        assert!(
            adapter
                .dispatch_ime(
                    &mut context,
                    document,
                    &ImeEvent::Preedit {
                        text: "你".into(),
                        selection: None,
                    },
                )
                .unwrap()
                .prevent_default
        );
        assert_eq!(
            context.world().ime(input.stable_id()).map(|ime| ime.text),
            Some("你")
        );
        assert!(
            adapter
                .dispatch_ime(&mut context, document, &ImeEvent::Commit("你".into()))
                .unwrap()
                .prevent_default
        );
        assert_eq!(context.world().text(input.stable_id()), Some("NanaU你"));
        assert_eq!(context.world().ime(input.stable_id()), None);
        assert!(
            adapter
                .dispatch(&mut context, document, &key("Backspace"))
                .unwrap()
                .prevent_default
        );
        assert_eq!(context.world().text(input.stable_id()), Some("NanaU"));
    }

    #[test]
    fn clipboard_shortcuts_move_text_between_the_editor_and_the_pasteboard() {
        let mut context = AppContext::new();
        let document = DocumentId::new(1).unwrap();
        let input = context
            .create_component(document, TextInput::new("Nana"))
            .unwrap();
        assert!(context.focus_node(document, input.stable_id()).unwrap());

        let clipboard = shared_clipboard(MemoryClipboard::new());
        let mut adapter = RuntimeInputAdapter::default().with_clipboard(Arc::clone(&clipboard));
        let primary = |key: &str| InputEvent::Keyboard {
            pressed: true,
            key: key.into(),
            text: None,
            code: key.into(),
            repeat: false,
            modifiers: InputModifiers {
                control: true,
                ..InputModifiers::default()
            },
        };

        // Nothing is selected yet, so a copy must not clear the pasteboard.
        assert!(
            !adapter
                .dispatch(&mut context, document, &primary("c"))
                .unwrap()
                .prevent_default
        );
        assert_eq!(clipboard.lock().unwrap().read_text(), None);

        assert!(
            adapter
                .dispatch(&mut context, document, &primary("a"))
                .unwrap()
                .prevent_default
        );
        assert!(
            adapter
                .dispatch(&mut context, document, &primary("x"))
                .unwrap()
                .prevent_default
        );
        assert_eq!(context.world().text(input.stable_id()), Some(""));
        assert_eq!(
            clipboard.lock().unwrap().read_text().as_deref(),
            Some("Nana")
        );

        assert!(
            adapter
                .dispatch(&mut context, document, &primary("v"))
                .unwrap()
                .prevent_default
        );
        assert!(
            adapter
                .dispatch(&mut context, document, &primary("v"))
                .unwrap()
                .prevent_default
        );
        assert_eq!(context.world().text(input.stable_id()), Some("NanaNana"));

        assert!(
            adapter
                .dispatch(&mut context, document, &primary("a"))
                .unwrap()
                .prevent_default
        );
        assert!(
            adapter
                .dispatch(&mut context, document, &primary("c"))
                .unwrap()
                .prevent_default
        );
        assert_eq!(context.world().text(input.stable_id()), Some("NanaNana"));
        assert_eq!(
            clipboard.lock().unwrap().read_text().as_deref(),
            Some("NanaNana")
        );
    }

    #[test]
    fn clipboard_probe_does_not_wait_for_busy_host_mutex() {
        let clipboard = shared_clipboard(MemoryClipboard::new());
        let guard = clipboard.lock().unwrap();
        let adapter = RuntimeInputAdapter::default().with_clipboard(Arc::clone(&clipboard));
        assert!(adapter.read_clipboard().is_none());
        drop(guard);
        assert!(adapter.read_clipboard().is_none());
    }

    fn document_text_pointer(phase: PointerPhase, x: f32, y: f32) -> InputEvent {
        InputEvent::Pointer {
            phase,
            pointer_id: 7,
            pointer_type: PointerType::Mouse,
            x,
            y,
            screen_x: x,
            screen_y: y,
            button: 0,
            buttons: u16::from(phase != PointerPhase::Up),
            pressure: 1.0,
            tangential_pressure: 0.0,
            tilt_x: 0,
            tilt_y: 0,
            twist: 0,
            is_primary: true,
            activation_click: false,
            modifiers: InputModifiers::default(),
        }
    }

    fn mount_document_text(
        context: &mut AppContext,
        value: &str,
        user_select: UserSelectSpec,
    ) -> (DocumentId, nana_ui_runtime::StableNodeId) {
        let document = DocumentId::new(1).unwrap();
        let mut style = NodeStyle::default();
        Arc::make_mut(&mut style.layout).user_select = Some(user_select);
        let label = context
            .create_component(document, Text::new(value).style(style))
            .unwrap();
        let node = label.stable_id();
        let mut layout = MutationQueue::new();
        layout.write_layout(
            node,
            LayoutBox {
                x: 0.0,
                y: 0.0,
                width: 400.0,
                height: 32.0,
            },
        );
        context.commit_mutations(layout).unwrap();
        context.resolve_styles(&[node]).unwrap();
        (document, node)
    }

    #[expect(
        clippy::too_many_arguments,
        reason = "test helper: fixture plus one pointer event"
    )]
    fn dispatch_document_pointer(
        adapter: &mut RuntimeInputAdapter,
        context: &mut AppContext,
        document: DocumentId,
        shaper: &mut MeasureTextShaper,
        phase: PointerPhase,
        x: f32,
        y: f32,
        millis: u64,
    ) {
        adapter
            .dispatch_with_shaper(
                context,
                document,
                &document_text_pointer(phase, x, y),
                Duration::from_millis(millis),
                Some(shaper),
            )
            .unwrap();
    }

    fn copy_shortcut() -> InputEvent {
        InputEvent::Keyboard {
            pressed: true,
            key: "c".into(),
            text: None,
            code: "c".into(),
            repeat: false,
            modifiers: InputModifiers {
                control: true,
                ..InputModifiers::default()
            },
        }
    }

    #[test]
    fn user_select_text_drag_copies_and_empty_or_none_leave_the_pasteboard() {
        let clipboard = shared_clipboard(MemoryClipboard::new());
        clipboard.lock().unwrap().write_text("keep-me");
        let primary = |key: &str| InputEvent::Keyboard {
            pressed: true,
            key: key.into(),
            text: None,
            code: key.into(),
            repeat: false,
            modifiers: InputModifiers {
                control: true,
                ..InputModifiers::default()
            },
        };

        let mut context = AppContext::new();
        let (document, node) =
            mount_document_text(&mut context, "Hello copy", UserSelectSpec::Text);
        let mut shaper = MeasureTextShaper;
        let mut adapter = RuntimeInputAdapter::default().with_clipboard(Arc::clone(&clipboard));
        dispatch_document_pointer(
            &mut adapter,
            &mut context,
            document,
            &mut shaper,
            PointerPhase::Down,
            2.0,
            16.0,
            1_000,
        );
        dispatch_document_pointer(
            &mut adapter,
            &mut context,
            document,
            &mut shaper,
            PointerPhase::Move,
            380.0,
            16.0,
            1_010,
        );
        dispatch_document_pointer(
            &mut adapter,
            &mut context,
            document,
            &mut shaper,
            PointerPhase::Up,
            380.0,
            16.0,
            1_020,
        );
        assert_eq!(
            context.document_selected_text(document).as_deref(),
            Some("Hello copy")
        );
        assert!(
            adapter
                .dispatch(&mut context, document, &primary("c"))
                .unwrap()
                .prevent_default
        );
        assert_eq!(
            clipboard.lock().unwrap().read_text().as_deref(),
            Some("Hello copy")
        );
        assert!(
            !adapter
                .dispatch(&mut context, document, &primary("x"))
                .unwrap()
                .prevent_default
        );
        assert_eq!(context.world().text(node), Some("Hello copy"));

        let mut none_context = AppContext::new();
        let (none_document, _) =
            mount_document_text(&mut none_context, "Hello copy", UserSelectSpec::None);
        clipboard.lock().unwrap().write_text("keep-me");
        let mut none_shaper = MeasureTextShaper;
        let mut none_adapter =
            RuntimeInputAdapter::default().with_clipboard(Arc::clone(&clipboard));
        dispatch_document_pointer(
            &mut none_adapter,
            &mut none_context,
            none_document,
            &mut none_shaper,
            PointerPhase::Down,
            2.0,
            16.0,
            2_000,
        );
        dispatch_document_pointer(
            &mut none_adapter,
            &mut none_context,
            none_document,
            &mut none_shaper,
            PointerPhase::Move,
            380.0,
            16.0,
            2_010,
        );
        dispatch_document_pointer(
            &mut none_adapter,
            &mut none_context,
            none_document,
            &mut none_shaper,
            PointerPhase::Up,
            380.0,
            16.0,
            2_020,
        );
        assert!(none_context.document_selected_text(none_document).is_none());
        assert!(
            !none_adapter
                .dispatch(&mut none_context, none_document, &primary("c"))
                .unwrap()
                .prevent_default
        );
        assert_eq!(
            clipboard.lock().unwrap().read_text().as_deref(),
            Some("keep-me")
        );

        let mut empty_context = AppContext::new();
        let (empty_document, _) =
            mount_document_text(&mut empty_context, "Hello copy", UserSelectSpec::Text);
        let mut empty_shaper = MeasureTextShaper;
        let mut empty_adapter =
            RuntimeInputAdapter::default().with_clipboard(Arc::clone(&clipboard));
        dispatch_document_pointer(
            &mut empty_adapter,
            &mut empty_context,
            empty_document,
            &mut empty_shaper,
            PointerPhase::Down,
            2.0,
            16.0,
            3_000,
        );
        dispatch_document_pointer(
            &mut empty_adapter,
            &mut empty_context,
            empty_document,
            &mut empty_shaper,
            PointerPhase::Up,
            2.0,
            16.0,
            3_010,
        );
        assert!(
            empty_context
                .document_selected_text(empty_document)
                .is_none()
        );
        assert!(
            !empty_adapter
                .dispatch(&mut empty_context, empty_document, &primary("c"))
                .unwrap()
                .prevent_default
        );
        assert_eq!(
            clipboard.lock().unwrap().read_text().as_deref(),
            Some("keep-me")
        );
    }

    #[test]
    fn user_select_all_click_copies_without_drag() {
        let clipboard = shared_clipboard(MemoryClipboard::new());
        clipboard.lock().unwrap().write_text("keep-me");
        let mut context = AppContext::new();
        let (document, _) = mount_document_text(&mut context, "Hello copy", UserSelectSpec::All);
        let mut shaper = MeasureTextShaper;
        let mut adapter = RuntimeInputAdapter::default().with_clipboard(Arc::clone(&clipboard));
        dispatch_document_pointer(
            &mut adapter,
            &mut context,
            document,
            &mut shaper,
            PointerPhase::Down,
            2.0,
            16.0,
            1_000,
        );
        dispatch_document_pointer(
            &mut adapter,
            &mut context,
            document,
            &mut shaper,
            PointerPhase::Up,
            2.0,
            16.0,
            1_010,
        );
        assert_eq!(
            context.document_selected_text(document).as_deref(),
            Some("Hello copy")
        );
        let copied = adapter
            .dispatch(&mut context, document, &copy_shortcut())
            .unwrap();
        assert!(copied.prevent_default);
        assert_eq!(
            clipboard.lock().unwrap().read_text().as_deref(),
            Some("Hello copy")
        );
    }

    #[test]
    fn a_read_only_field_copies_but_never_loses_text_to_a_cut() {
        let mut context = AppContext::new();
        let document = DocumentId::new(1).unwrap();
        let input = context
            .create_component(document, TextInput::new("Nana").read_only(true))
            .unwrap();
        assert!(context.focus_node(document, input.stable_id()).unwrap());

        let clipboard = shared_clipboard(MemoryClipboard::new());
        let mut adapter = RuntimeInputAdapter::default().with_clipboard(Arc::clone(&clipboard));
        let primary = |key: &str| InputEvent::Keyboard {
            pressed: true,
            key: key.into(),
            text: None,
            code: key.into(),
            repeat: false,
            modifiers: InputModifiers {
                meta: true,
                ..InputModifiers::default()
            },
        };

        assert!(
            adapter
                .dispatch(&mut context, document, &primary("a"))
                .unwrap()
                .prevent_default
        );
        assert!(
            adapter
                .dispatch(&mut context, document, &primary("x"))
                .unwrap()
                .prevent_default
        );
        assert_eq!(
            clipboard.lock().unwrap().read_text().as_deref(),
            Some("Nana")
        );
        assert_eq!(context.world().text(input.stable_id()), Some("Nana"));

        assert!(clipboard.lock().unwrap().write_text("pasted"));
        assert!(
            !adapter
                .dispatch(&mut context, document, &primary("v"))
                .unwrap()
                .prevent_default
        );
        assert_eq!(context.world().text(input.stable_id()), Some("Nana"));
    }

    #[test]
    fn focused_runtime_text_inserts_shifted_printable_characters() {
        let mut context = AppContext::new();
        let document = DocumentId::new(1).unwrap();
        let area = context
            .create_component(document, TextArea::new("inspect "))
            .unwrap();
        assert!(context.focus_node(document, area.stable_id()).unwrap());
        let event = InputEvent::Keyboard {
            pressed: true,
            key: "2".into(),
            text: Some("@".into()),
            code: "Digit2".into(),
            repeat: false,
            modifiers: InputModifiers {
                shift: true,
                ..InputModifiers::default()
            },
        };
        assert!(
            RuntimeInputAdapter::default()
                .dispatch(&mut context, document, &event)
                .unwrap()
                .prevent_default
        );
        assert_eq!(context.world().text(area.stable_id()), Some("inspect @"));
    }

    #[test]
    fn focused_runtime_textarea_ime_updates_multiline_state() {
        let mut context = AppContext::new();
        let document = DocumentId::new(1).unwrap();
        let area = context
            .create_component(document, TextArea::new("第一行\n"))
            .unwrap();
        assert!(context.focus_node(document, area.stable_id()).unwrap());

        let adapter = RuntimeInputAdapter::default();
        assert!(
            adapter
                .dispatch_ime(
                    &mut context,
                    document,
                    &ImeEvent::Preedit {
                        text: "第二".into(),
                        selection: Some((0, "第".len())),
                    },
                )
                .unwrap()
                .prevent_default
        );
        let composition = context
            .world()
            .ime(area.stable_id())
            .expect("focused textarea keeps preedit on retained state");
        assert_eq!(composition.text, "第二");
        assert_eq!(composition.selection, Some((0, "第".len())));
        assert_eq!(context.world().text(area.stable_id()), Some("第一行\n"));

        assert!(
            adapter
                .dispatch_ime(&mut context, document, &ImeEvent::Commit("第二行".into()))
                .unwrap()
                .prevent_default
        );
        assert_eq!(
            context.world().text(area.stable_id()),
            Some("第一行\n第二行")
        );
        assert_eq!(context.world().ime(area.stable_id()), None);

        context
            .update_component(area, |area, _cx| area.disabled = true)
            .unwrap();
        assert!(
            !adapter
                .dispatch_ime(
                    &mut context,
                    document,
                    &ImeEvent::Preedit {
                        text: "三".into(),
                        selection: None,
                    },
                )
                .unwrap()
                .prevent_default
        );
        assert_eq!(
            context.world().text(area.stable_id()),
            Some("第一行\n第二行")
        );
        assert_eq!(context.world().ime(area.stable_id()), None);
    }

    #[test]
    fn dispatch_ime_commits_a_focused_text_input_without_a_typed_view() {
        let mut context = AppContext::new();
        let (document, id) = focused_untyped_text_input(&mut context, "Nana");
        let adapter = RuntimeInputAdapter::default();
        assert!(
            adapter
                .dispatch_ime(
                    &mut context,
                    document,
                    &ImeEvent::Preedit {
                        text: "世".into(),
                        selection: Some((0, "世".len())),
                    },
                )
                .unwrap()
                .prevent_default
        );
        assert_eq!(context.world().ime(id).map(|ime| ime.text), Some("世"));
        assert_eq!(
            context.world().text_input(id).map(|state| state.value),
            Some("Nana")
        );

        assert!(
            adapter
                .dispatch_ime(&mut context, document, &ImeEvent::Commit("世界".into()))
                .unwrap()
                .prevent_default
        );
        assert_eq!(
            context.world().text_input(id).map(|state| state.value),
            Some("Nana世界")
        );
        assert!(context.world().ime(id).is_none());
    }

    #[test]
    fn dispatch_ime_disabled_commits_leftover_preedit_without_a_typed_view() {
        let mut context = AppContext::new();
        let (document, id) = focused_untyped_text_input(&mut context, "Nana");
        let adapter = RuntimeInputAdapter::default();
        assert!(
            adapter
                .dispatch_ime(
                    &mut context,
                    document,
                    &ImeEvent::Preedit {
                        text: "世".into(),
                        selection: Some((0, "世".len())),
                    },
                )
                .unwrap()
                .prevent_default
        );
        assert!(
            adapter
                .dispatch_ime(&mut context, document, &ImeEvent::Disabled)
                .unwrap()
                .prevent_default
        );
        assert_eq!(
            context.world().text_input(id).map(|state| state.value),
            Some("Nana世")
        );
        assert!(context.world().ime(id).is_none());
    }

    #[test]
    fn dispatch_ime_cancelled_discards_leftover_preedit_without_commit() {
        let mut context = AppContext::new();
        let (document, id) = focused_untyped_text_input(&mut context, "Nana");
        let adapter = RuntimeInputAdapter::default();
        adapter
            .dispatch_ime(
                &mut context,
                document,
                &ImeEvent::Preedit {
                    text: "世".into(),
                    selection: Some((0, "世".len())),
                },
            )
            .unwrap();
        assert!(
            adapter
                .dispatch_ime(&mut context, document, &ImeEvent::Cancelled)
                .unwrap()
                .prevent_default
        );
        assert_eq!(
            context.world().text_input(id).map(|state| state.value),
            Some("Nana")
        );
        assert!(context.world().ime(id).is_none());
    }

    #[test]
    fn dispatch_ime_deletes_surrounding_committed_text_and_skips_invalid_spans() {
        let mut context = AppContext::new();
        let (document, id) = focused_untyped_text_input(&mut context, "你好");
        let adapter = RuntimeInputAdapter::default();
        assert!(
            adapter
                .dispatch_ime(
                    &mut context,
                    document,
                    &ImeEvent::Preedit {
                        text: "世".into(),
                        selection: Some((0, "世".len())),
                    },
                )
                .unwrap()
                .prevent_default
        );
        assert!(
            adapter
                .dispatch_ime(
                    &mut context,
                    document,
                    &ImeEvent::DeleteSurrounding {
                        before_bytes: "好".len(),
                        after_bytes: 0,
                    },
                )
                .unwrap()
                .prevent_default
        );
        assert_eq!(
            context.world().text_input(id).map(|state| state.value),
            Some("你")
        );
        assert_eq!(
            context.world().ime(id).map(|ime| ime.text),
            Some("世"),
            "delete surrounding must not clear preedit"
        );

        assert!(
            adapter
                .dispatch_ime(
                    &mut context,
                    document,
                    &ImeEvent::DeleteSurrounding {
                        before_bytes: 1,
                        after_bytes: 0,
                    },
                )
                .unwrap()
                .prevent_default,
            "focused editable still consumes an un-applicable span"
        );
        assert_eq!(
            context.world().text_input(id).map(|state| state.value),
            Some("你"),
            "invalid byte span must leave committed text unchanged"
        );
    }

    #[test]
    fn wheel_routes_to_nearest_scrollview_and_bubbles_at_edge() {
        let mut context = AppContext::new();
        let document = DocumentId::new(1).unwrap();
        let outer = context
            .create_component(document, ScrollView::new(ScrollAxes::Vertical))
            .unwrap();
        let inner = context
            .create_component(document, ScrollView::new(ScrollAxes::Vertical))
            .unwrap();
        let cell = context
            .create_component(document, TableCell::new("row"))
            .unwrap();
        // Beside the inner scrollport, content reaching 300 down the outer.
        let tail = context
            .create_component(document, TableCell::new("tail"))
            .unwrap();
        context.append_child(outer, inner).unwrap();
        context.append_child(outer, tail).unwrap();
        context.append_child(inner, cell).unwrap();
        let mut layout = MutationQueue::new();
        layout.write_layout(
            outer.stable_id(),
            LayoutBox {
                x: 0.0,
                y: 0.0,
                width: 200.0,
                height: 100.0,
            },
        );
        layout.write_layout(
            inner.stable_id(),
            LayoutBox {
                x: 0.0,
                y: 0.0,
                width: 180.0,
                height: 80.0,
            },
        );
        // 140 of content in the inner's 80: it scrolls 60, the outer 200.
        layout.write_layout(
            cell.stable_id(),
            LayoutBox {
                x: 0.0,
                y: 0.0,
                width: 160.0,
                height: 140.0,
            },
        );
        layout.write_layout(
            tail.stable_id(),
            LayoutBox {
                x: 190.0,
                y: 0.0,
                width: 10.0,
                height: 300.0,
            },
        );
        context.commit_mutations(layout).unwrap();
        context.take_system_work();
        context.rebuild_hit_test(document);

        let mut adapter = RuntimeInputAdapter::default();
        assert!(
            adapter
                .dispatch(&mut context, document, &wheel(10.0, 10.0, -1.0))
                .unwrap()
                .prevent_default
        );
        assert_eq!(
            context.world().scroll_offset(inner.stable_id()).unwrap().y,
            60.0
        );
        assert_eq!(
            context.world().scroll_offset(outer.stable_id()).unwrap().y,
            0.0
        );
        context.take_system_work();
        context.rebuild_hit_test(document);

        assert!(
            adapter
                .dispatch(&mut context, document, &wheel(10.0, 10.0, -1.0))
                .unwrap()
                .prevent_default
        );
        assert_eq!(
            context.world().scroll_offset(inner.stable_id()).unwrap().y,
            60.0
        );
        assert_eq!(
            context.world().scroll_offset(outer.stable_id()).unwrap().y,
            60.0
        );
    }

    #[test]
    fn wheel_on_overflow_auto_updates_scroll_offset() {
        let mut context = AppContext::new();
        let document = DocumentId::new(1).unwrap();
        let scroller = nana_ui_runtime::StableNodeId::new(1).unwrap();
        let child = nana_ui_runtime::StableNodeId::new(2).unwrap();
        let mut create = MutationQueue::new();
        create.create(scroller, document, NodeKind::Element { tag: "div".into() });
        create.create(child, document, NodeKind::Element { tag: "item".into() });
        create.insert(scroller, child, None);
        create.set_style(
            scroller,
            NodeStyle {
                layout: Arc::new(LayoutStyle {
                    overflow_y: OverflowSpec::Auto,
                    ..LayoutStyle::default()
                }),
                ..NodeStyle::default()
            },
        );
        create.write_layout(
            scroller,
            LayoutBox {
                x: 0.0,
                y: 0.0,
                width: 200.0,
                height: 100.0,
            },
        );
        create.write_layout(
            child,
            LayoutBox {
                x: 0.0,
                y: 0.0,
                width: 200.0,
                height: 300.0,
            },
        );
        context.commit_mutations(create).unwrap();
        let work = context.take_system_work();
        context.resolve_styles(&work.style).unwrap();
        context.rebuild_hit_test(document);

        let mut adapter = RuntimeInputAdapter::default();
        assert!(
            adapter
                .dispatch(&mut context, document, &wheel(10.0, 10.0, -1.0))
                .unwrap()
                .prevent_default
        );
        assert_eq!(context.world().scroll_offset(scroller).unwrap().y, 60.0);
        assert_eq!(context.world().scroll_offset(child).unwrap().y, 0.0);
        assert!(
            context
                .world()
                .node_style(scroller)
                .unwrap()
                .layout
                .overflow_y
                .scrolls()
        );
        assert!(
            !context.is_scroll_view(scroller),
            "L1 overflow must not stamp a ScrollView"
        );
    }

    #[test]
    fn keyboard_routes_navigation_from_focused_table_cell() {
        let mut context = AppContext::new();
        let document = DocumentId::new(1).unwrap();
        let table = context.create_component(document, Table::new()).unwrap();
        let first_row = context.create_component(document, TableRow::new()).unwrap();
        let second_row = context.create_component(document, TableRow::new()).unwrap();
        let first = context
            .create_component(document, TableCell::new("one"))
            .unwrap();
        let second = context
            .create_component(document, TableCell::new("two"))
            .unwrap();
        context.append_child(table, first_row).unwrap();
        context.append_child(table, second_row).unwrap();
        context.append_child(first_row, first).unwrap();
        context.append_child(second_row, second).unwrap();
        let mut focus = MutationQueue::new();
        focus.request_focus(document, Some(first.stable_id()));
        context.commit_mutations(focus).unwrap();

        let event = InputEvent::Keyboard {
            pressed: true,
            key: "ArrowDown".into(),
            text: None,
            code: "ArrowDown".into(),
            repeat: false,
            modifiers: InputModifiers::default(),
        };
        assert!(
            RuntimeInputAdapter::default()
                .dispatch(&mut context, document, &event)
                .unwrap()
                .prevent_default
        );
        assert_eq!(context.world().focused(document), Some(second.stable_id()));
    }

    #[test]
    fn segmented_keyboard_routing_precedes_generic_navigation_and_activation() {
        let mut context = AppContext::new();
        let document = DocumentId::new(1).unwrap();
        let control = context
            .create_component(document, SegmentedControl::new())
            .unwrap();
        let first = context
            .create_detached_component(document, SegmentedOption::new("Code"))
            .unwrap();
        let disabled = context
            .create_detached_component(document, SegmentedOption::new("Split").disabled(true))
            .unwrap();
        let last = context
            .create_detached_component(document, SegmentedOption::new("Preview"))
            .unwrap();
        context
            .set_segmented_options(control, vec![first, disabled, last], Some(first))
            .unwrap();
        let requests = Arc::new(Mutex::new(Vec::new()));
        let observed = Arc::clone(&requests);
        context
            .on(
                control,
                move |_control, event: &SegmentedSelectionRequested, _cx| {
                    observed.lock().unwrap().push(event.option);
                },
            )
            .unwrap();
        context.focus_node(document, first.stable_id()).unwrap();
        let key = |key: &str, repeat: bool, modifiers: InputModifiers| InputEvent::Keyboard {
            pressed: true,
            key: key.into(),
            text: None,
            code: key.into(),
            repeat,
            modifiers,
        };
        let mut adapter = RuntimeInputAdapter::default();
        assert!(
            adapter
                .dispatch(
                    &mut context,
                    document,
                    &key("ArrowRight", false, InputModifiers::default()),
                )
                .unwrap()
                .prevent_default
        );
        assert_eq!(context.world().focused(document), Some(last.stable_id()));
        assert_eq!(
            context.read(control, SegmentedControl::selected).unwrap(),
            Some(first.stable_id())
        );
        assert_eq!(&*requests.lock().unwrap(), &[last.stable_id()]);
        for modifiers in [
            InputModifiers {
                alt: true,
                ..InputModifiers::default()
            },
            InputModifiers {
                control: true,
                ..InputModifiers::default()
            },
            InputModifiers {
                shift: true,
                ..InputModifiers::default()
            },
            InputModifiers {
                meta: true,
                ..InputModifiers::default()
            },
        ] {
            assert!(
                !adapter
                    .dispatch(&mut context, document, &key("Home", true, modifiers))
                    .unwrap()
                    .prevent_default
            );
        }
        assert_eq!(context.world().focused(document), Some(last.stable_id()));
        assert_eq!(requests.lock().unwrap().as_slice(), [last.stable_id()]);
        assert!(
            adapter
                .dispatch(
                    &mut context,
                    document,
                    &key("Home", true, InputModifiers::default()),
                )
                .unwrap()
                .prevent_default
        );
        assert_eq!(context.world().focused(document), Some(first.stable_id()));
        assert_eq!(
            requests.lock().unwrap().as_slice(),
            [last.stable_id(), first.stable_id()]
        );
        let count = requests.lock().unwrap().len();
        assert!(
            adapter
                .dispatch(
                    &mut context,
                    document,
                    &key("Space", true, InputModifiers::default()),
                )
                .unwrap()
                .prevent_default
        );
        assert_eq!(requests.lock().unwrap().len(), count);
        assert!(
            adapter
                .dispatch(
                    &mut context,
                    document,
                    &key("Enter", false, InputModifiers::default()),
                )
                .unwrap()
                .prevent_default
        );
        assert_eq!(requests.lock().unwrap().last(), Some(&first.stable_id()));
        assert_eq!(
            context.read(control, SegmentedControl::selected).unwrap(),
            Some(first.stable_id())
        );
        assert!(
            context
                .set_segmented_selection(control, Some(last))
                .unwrap()
        );
        assert_eq!(
            context.read(control, SegmentedControl::selected).unwrap(),
            Some(last.stable_id())
        );
    }

    #[test]
    fn range_field_quantizes_keyboard_and_cancels_captured_drag() {
        let mut context = AppContext::new();
        let document = DocumentId::new(1).unwrap();
        let range = context
            .create_component(document, RangeField::new(0.5, 0.0, 1.0, 0.1))
            .unwrap();
        let mut layout = MutationQueue::new();
        layout.write_layout(
            range.stable_id(),
            LayoutBox {
                x: 10.0,
                y: 10.0,
                width: 300.0,
                height: 32.0,
            },
        );
        context.commit_mutations(layout).unwrap();
        context.rebuild_hit_test(document);
        assert!(context.focus_node(document, range.stable_id()).unwrap());

        let key = |key: &str| InputEvent::Keyboard {
            pressed: true,
            key: key.into(),
            text: None,
            code: key.into(),
            repeat: false,
            modifiers: InputModifiers::default(),
        };
        let mut adapter = RuntimeInputAdapter::default();
        assert!(
            adapter
                .dispatch(&mut context, document, &key("ArrowRight"))
                .unwrap()
                .prevent_default
        );
        assert!((context.read(range, |range| range.value).unwrap() - 0.6).abs() < 1e-12);
        assert!(
            adapter
                .dispatch(&mut context, document, &key("Home"))
                .unwrap()
                .prevent_default
        );
        assert_eq!(context.read(range, |range| range.value).unwrap(), 0.0);

        let track = match context.world().component_geometry(range.stable_id()) {
            Some(nana_ui_runtime::ComponentGeometry::Range { track, .. }) => track,
            _ => panic!("range geometry expected"),
        };
        let drag_x = track.x + track.width * 0.8;
        assert!(
            adapter
                .dispatch(
                    &mut context,
                    document,
                    &pointer(PointerPhase::Down, drag_x, 20.0)
                )
                .unwrap()
                .prevent_default
        );
        assert_eq!(context.read(range, |range| range.value).unwrap(), 0.8);
        assert!(
            adapter
                .dispatch(
                    &mut context,
                    document,
                    &pointer(PointerPhase::Cancel, drag_x, 20.0)
                )
                .unwrap()
                .prevent_default
        );
        assert_eq!(context.read(range, |range| range.value).unwrap(), 0.0);
        assert_eq!(context.world().pointer_capture(document, 1), None);
    }

    #[test]
    fn a_pointer_drag_on_a_range_commits_once_on_release() {
        let mut context = AppContext::new();
        let document = DocumentId::new(1).unwrap();
        let range = context
            .create_component(document, RangeField::new(0.0, 0.0, 1.0, 0.1))
            .unwrap();
        let mut layout = MutationQueue::new();
        layout.write_layout(
            range.stable_id(),
            LayoutBox {
                x: 10.0,
                y: 10.0,
                width: 300.0,
                height: 32.0,
            },
        );
        context.commit_mutations(layout).unwrap();
        context.rebuild_hit_test(document);
        let previews = Arc::new(Mutex::new(Vec::new()));
        let observed = Arc::clone(&previews);
        context
            .on(range, move |_, event: &nana_ui_runtime::RangeInput, _| {
                observed.lock().unwrap().push(event.value);
            })
            .unwrap();
        let commits = Arc::new(Mutex::new(Vec::new()));
        let observed = Arc::clone(&commits);
        context
            .on(range, move |_, event: &nana_ui_runtime::RangeChanged, _| {
                observed.lock().unwrap().push(event.value);
            })
            .unwrap();
        let track = match context.world().component_geometry(range.stable_id()) {
            Some(ComponentGeometry::Range { track, .. }) => track,
            _ => panic!("range geometry expected"),
        };
        let mut adapter = RuntimeInputAdapter::default();
        for (phase, fraction) in [
            (PointerPhase::Down, 0.2),
            (PointerPhase::Move, 0.5),
            (PointerPhase::Move, 0.7),
            (PointerPhase::Up, 0.7),
        ] {
            adapter
                .dispatch(
                    &mut context,
                    document,
                    &pointer(phase, track.x + track.width * fraction, 20.0),
                )
                .unwrap();
        }
        assert_eq!(previews.lock().unwrap().len(), 3);
        let commits = commits.lock().unwrap();
        assert_eq!(commits.len(), 1, "one commit per drag: {commits:?}");
        assert!((commits[0] - 0.7).abs() < 1e-9);
        assert_eq!(context.world().pointer_capture(document, 1), None);
    }

    #[test]
    fn overlay_pointer_sequence_never_activates_the_underlay() {
        let mut context = AppContext::new();
        let document = DocumentId::new(1).unwrap();
        let underlay = context
            .create_component(document, Button::new("Underlay"))
            .unwrap();
        let host = context
            .create_component(document, OverlayHost::new())
            .unwrap();
        let dialog = context
            .create_component(document, Dialog::new("Dialog"))
            .unwrap();
        context.append_child(host, dialog).unwrap();
        let activations = Arc::new(Mutex::new(0));
        let observed = Arc::clone(&activations);
        context
            .on(underlay, move |_button, _event: &Activate, _cx| {
                *observed.lock().unwrap() += 1;
            })
            .unwrap();
        let mut layout = MutationQueue::new();
        layout.write_layout(
            underlay.stable_id(),
            LayoutBox {
                x: 0.0,
                y: 0.0,
                width: 300.0,
                height: 300.0,
            },
        );
        layout.write_layout(
            dialog.stable_id(),
            LayoutBox {
                x: 100.0,
                y: 100.0,
                width: 100.0,
                height: 100.0,
            },
        );
        context.commit_mutations(layout).unwrap();
        context.activate_overlay(host, dialog).unwrap();
        context.rebuild_hit_test(document);

        let mut adapter = RuntimeInputAdapter::default();
        assert!(
            adapter
                .dispatch(
                    &mut context,
                    document,
                    &pointer(PointerPhase::Down, 20.0, 20.0),
                )
                .unwrap()
                .prevent_default
        );
        assert!(
            adapter
                .dispatch(
                    &mut context,
                    document,
                    &pointer(PointerPhase::Up, 20.0, 20.0),
                )
                .unwrap()
                .prevent_default
        );
        assert_eq!(*activations.lock().unwrap(), 0);
    }

    /// Lays a popover over a button so the two never overlap, and reports the
    /// shared activation counter of the button.
    fn popover_over_button(
        context: &mut AppContext,
        document: DocumentId,
    ) -> (Entity<ActionMenu>, Arc<Mutex<u32>>) {
        let underlay = context
            .create_component(document, Button::new("Underlay"))
            .unwrap();
        let menu = context
            .create_component(document, ActionMenu::new().open(true))
            .unwrap();
        let activations = Arc::new(Mutex::new(0));
        let observed = Arc::clone(&activations);
        context
            .on(underlay, move |_button, _event: &Activate, _cx| {
                *observed.lock().unwrap() += 1;
            })
            .unwrap();
        let mut layout = MutationQueue::new();
        layout.write_layout(
            underlay.stable_id(),
            LayoutBox {
                x: 0.0,
                y: 0.0,
                width: 300.0,
                height: 60.0,
            },
        );
        layout.write_layout(
            menu.stable_id(),
            LayoutBox {
                x: 0.0,
                y: 100.0,
                width: 200.0,
                height: 100.0,
            },
        );
        context.commit_mutations(layout).unwrap();
        context.rebuild_hit_test(document);
        (menu, activations)
    }

    #[test]
    fn outside_press_closes_the_popover_without_reaching_the_underlay() {
        let mut context = AppContext::new();
        let document = DocumentId::new(1).unwrap();
        let (menu, activations) = popover_over_button(&mut context, document);
        let mut adapter = RuntimeInputAdapter::default();

        assert!(
            adapter
                .dispatch(
                    &mut context,
                    document,
                    &pointer(PointerPhase::Down, 20.0, 20.0),
                )
                .unwrap()
                .prevent_default
        );
        adapter
            .dispatch(
                &mut context,
                document,
                &pointer(PointerPhase::Up, 20.0, 20.0),
            )
            .unwrap();

        assert!(!context.read(menu, |menu| menu.popover.open).unwrap());
        // An app-owned trigger button sits outside the popover too, so letting
        // this press through would toggle the menu straight back open.
        assert_eq!(*activations.lock().unwrap(), 0);
    }

    #[test]
    fn press_inside_the_popover_leaves_it_open() {
        let mut context = AppContext::new();
        let document = DocumentId::new(1).unwrap();
        let (menu, _) = popover_over_button(&mut context, document);
        let item = context
            .create_component(document, ActionMenuItem::new("Rename"))
            .unwrap();
        context.append_child(menu, item).unwrap();
        let mut layout = MutationQueue::new();
        layout.write_layout(
            item.stable_id(),
            LayoutBox {
                x: 4.0,
                y: 104.0,
                width: 192.0,
                height: 28.0,
            },
        );
        context.commit_mutations(layout).unwrap();
        context.rebuild_hit_test(document);
        let mut adapter = RuntimeInputAdapter::default();

        adapter
            .dispatch(
                &mut context,
                document,
                &pointer(PointerPhase::Down, 20.0, 110.0),
            )
            .unwrap();

        assert!(context.read(menu, |menu| menu.popover.open).unwrap());
    }

    #[test]
    fn escape_closes_focused_field_options_without_committing() {
        use nana_ui_core::DropdownEvent;
        use nana_ui_runtime::{
            Dropdown, DropdownOption, SearchDropdown, SearchDropdownEvent, SearchDropdownOption,
            Select, SelectOption,
        };

        let mut context = AppContext::new();
        let document = DocumentId::new(1).unwrap();
        let select = context
            .create_component(
                document,
                Select::new(Some("a"))
                    .options([
                        SelectOption::new("a", "Alpha"),
                        SelectOption::new("b", "Beta"),
                    ])
                    .opened(true),
            )
            .unwrap();
        let dropdown = context
            .create_component(
                document,
                Dropdown::single(Some("a"))
                    .options([
                        DropdownOption::new("a", "Alpha"),
                        DropdownOption::new("b", "Beta"),
                    ])
                    .opened(true),
            )
            .unwrap();
        let search = context
            .create_component(
                document,
                SearchDropdown::new(Some("a"))
                    .options([
                        SearchDropdownOption::new("a", "Alpha"),
                        SearchDropdownOption::new("b", "Beta"),
                    ])
                    .query("Beta")
                    .opened(true),
            )
            .unwrap();
        let dropdown_events = Arc::new(Mutex::new(Vec::new()));
        let events = Arc::clone(&dropdown_events);
        context
            .on(dropdown, move |_, event: &DropdownEvent<Arc<str>>, _| {
                events.lock().unwrap().push(event.clone());
            })
            .unwrap();
        let search_events = Arc::new(Mutex::new(Vec::new()));
        let events = Arc::clone(&search_events);
        context
            .on(search, move |_, event: &SearchDropdownEvent, _| {
                events.lock().unwrap().push(event.clone());
            })
            .unwrap();
        context
            .update_component(select, |field, _| field.highlighted = Some(1))
            .unwrap();
        context
            .update_component(dropdown, |field, _| field.highlighted = Some(1))
            .unwrap();
        let selection = context
            .read(dropdown, |field| field.selection.clone())
            .unwrap();
        let search_state = context.read(search, |field| field.state.clone()).unwrap();
        let mut adapter = RuntimeInputAdapter::default();
        let escape = InputEvent::Keyboard {
            pressed: true,
            key: "Escape".into(),
            text: None,
            code: "Escape".into(),
            repeat: false,
            modifiers: InputModifiers::default(),
        };
        for target in [select.stable_id(), dropdown.stable_id(), search.stable_id()] {
            assert!(context.focus_node(document, target).unwrap());
            assert!(
                adapter
                    .dispatch(&mut context, document, &escape)
                    .unwrap()
                    .prevent_default
            );
            assert!(
                !adapter
                    .dispatch(&mut context, document, &escape)
                    .unwrap()
                    .prevent_default
            );
        }
        assert_eq!(
            context
                .read(select, |field| (field.opened, field.value.clone()))
                .unwrap(),
            (false, Some(Arc::from("a")))
        );
        assert_eq!(
            context
                .read(dropdown, |field| (field.opened, field.selection.clone()))
                .unwrap(),
            (false, selection)
        );
        assert_eq!(
            context
                .read(search, |field| (
                    field.opened,
                    field.value.clone(),
                    field.query.clone(),
                    field.state.clone()
                ))
                .unwrap(),
            (false, Some(Arc::from("a")), "Beta".into(), search_state)
        );
        assert_eq!(
            *dropdown_events.lock().unwrap(),
            vec![DropdownEvent::Closed]
        );
        assert_eq!(
            *search_events.lock().unwrap(),
            vec![SearchDropdownEvent::Closed]
        );
    }

    #[test]
    fn escape_closes_the_popover() {
        let mut context = AppContext::new();
        let document = DocumentId::new(1).unwrap();
        let (menu, _) = popover_over_button(&mut context, document);
        let mut adapter = RuntimeInputAdapter::default();
        let escape = InputEvent::Keyboard {
            pressed: true,
            key: "Escape".into(),
            text: None,
            code: "Escape".into(),
            repeat: false,
            modifiers: InputModifiers::default(),
        };

        assert!(
            adapter
                .dispatch(&mut context, document, &escape)
                .unwrap()
                .prevent_default
        );
        assert!(!context.read(menu, |menu| menu.popover.open).unwrap());
        // The next Escape belongs to the application navigation layer. A
        // host must pass the per-event result rather than cache overlay state.
        assert!(
            !adapter
                .dispatch(&mut context, document, &escape)
                .unwrap()
                .prevent_default
        );
    }

    #[test]
    fn a_popover_that_opts_out_ignores_outside_presses_and_escape() {
        let mut context = AppContext::new();
        let document = DocumentId::new(1).unwrap();
        let (menu, _) = popover_over_button(&mut context, document);
        context
            .update_component(menu, |menu, _| {
                menu.popover.close_on_outside = false;
                menu.popover.close_on_escape = false;
            })
            .unwrap();
        let mut adapter = RuntimeInputAdapter::default();

        adapter
            .dispatch(
                &mut context,
                document,
                &pointer(PointerPhase::Down, 20.0, 20.0),
            )
            .unwrap();
        adapter
            .dispatch(
                &mut context,
                document,
                &InputEvent::Keyboard {
                    pressed: true,
                    key: "Escape".into(),
                    text: None,
                    code: "Escape".into(),
                    repeat: false,
                    modifiers: InputModifiers::default(),
                },
            )
            .unwrap();

        assert!(context.read(menu, |menu| menu.popover.open).unwrap());
    }

    #[test]
    fn menu_item_can_close_during_activation_without_releasing_input_or_wheel_barrier() {
        let mut context = AppContext::new();
        let document = DocumentId::new(1).unwrap();
        let background = context
            .create_component(document, ScrollView::new(ScrollAxes::Vertical))
            .unwrap();
        let host = context
            .create_component(document, OverlayHost::new())
            .unwrap();
        let menu = context
            .create_component(document, ActionMenu::new().open(true))
            .unwrap();
        let item = context
            .create_component(document, ActionMenuItem::new("Build"))
            .unwrap();
        context.append_child(background, host).unwrap();
        context.append_child(host, menu).unwrap();
        context.append_child(menu, item).unwrap();
        let host_id = host.stable_id();
        context
            .on(item, move |_item, _event: &Activate, cx| {
                cx.mutations()
                    .set_overlay_host(host_id, OverlayHostState::default());
            })
            .unwrap();
        let mut layout = MutationQueue::new();
        for (id, x, y, width, height) in [
            (background.stable_id(), 0.0, 0.0, 300.0, 300.0),
            (menu.stable_id(), 100.0, 100.0, 100.0, 100.0),
            (item.stable_id(), 110.0, 110.0, 80.0, 32.0),
        ] {
            layout.write_layout(
                id,
                LayoutBox {
                    x,
                    y,
                    width,
                    height,
                },
            );
        }
        context.commit_mutations(layout).unwrap();
        context
            .set_scroll_metrics(
                background,
                ScrollMetrics {
                    viewport_width: 300.0,
                    viewport_height: 300.0,
                    content_width: 300.0,
                    content_height: 900.0,
                    origin_x: 0.0,
                    origin_y: 0.0,
                },
            )
            .unwrap();
        context.activate_overlay(host, menu).unwrap();
        let work = context.take_system_work();
        context.resolve_styles(&work.style).unwrap();
        context.rebuild_hit_test(document);
        let mut adapter = RuntimeInputAdapter::default();

        assert!(
            adapter
                .dispatch(&mut context, document, &wheel(20.0, 20.0, -1.0))
                .unwrap()
                .prevent_default
        );
        assert_eq!(
            context
                .world()
                .scroll_offset(background.stable_id())
                .unwrap()
                .y,
            0.0
        );
        assert!(
            adapter
                .dispatch(
                    &mut context,
                    document,
                    &pointer(PointerPhase::Down, 120.0, 120.0),
                )
                .unwrap()
                .prevent_default
        );
        assert!(
            adapter
                .dispatch(
                    &mut context,
                    document,
                    &pointer(PointerPhase::Up, 120.0, 120.0),
                )
                .unwrap()
                .prevent_default
        );
        assert_eq!(
            context
                .world()
                .overlay_host(host.stable_id())
                .unwrap()
                .active,
            None
        );
    }

    #[test]
    fn overlay_keyboard_ignores_primary_tab_and_repeated_escape() {
        let mut context = AppContext::new();
        let document = DocumentId::new(1).unwrap();
        let host = context
            .create_component(document, OverlayHost::new())
            .unwrap();
        let dialog = context
            .create_component(document, Dialog::new("Settings"))
            .unwrap();
        let button = context
            .create_detached_component(document, Button::new("Save"))
            .unwrap();
        context.append_child(host, dialog).unwrap();
        context
            .set_modal_slots(
                dialog,
                ModalSlots {
                    actions: vec![button.stable_id()],
                    ..ModalSlots::default()
                },
            )
            .unwrap();
        context.activate_overlay(host, dialog).unwrap();
        let mut adapter = RuntimeInputAdapter::default();
        let key = |key: &str, repeat: bool, modifiers: InputModifiers| InputEvent::Keyboard {
            pressed: true,
            key: key.into(),
            text: None,
            code: key.into(),
            repeat,
            modifiers,
        };

        let primary_tab = key(
            "Tab",
            false,
            InputModifiers {
                control: true,
                ..InputModifiers::default()
            },
        );
        assert!(
            adapter
                .dispatch(&mut context, document, &primary_tab)
                .unwrap()
                .prevent_default
        );
        let repeat_escape = key("Escape", true, InputModifiers::default());
        assert!(
            adapter
                .dispatch(&mut context, document, &repeat_escape)
                .unwrap()
                .prevent_default
        );
        assert_eq!(
            context
                .world()
                .overlay_host(host.stable_id())
                .unwrap()
                .active,
            Some(dialog.stable_id())
        );
        let key_release = InputEvent::Keyboard {
            pressed: false,
            key: "a".into(),
            text: None,
            code: "KeyA".into(),
            repeat: false,
            modifiers: InputModifiers::default(),
        };
        assert!(
            adapter
                .dispatch(&mut context, document, &key_release)
                .unwrap()
                .prevent_default
        );
        let escape = key("Escape", false, InputModifiers::default());
        assert!(
            adapter
                .dispatch(&mut context, document, &escape)
                .unwrap()
                .prevent_default
        );
        assert!(context.active_runtime_overlay(document).is_none());
        context.advance_animations(std::time::Duration::from_secs(1));
        assert_eq!(
            context
                .world()
                .overlay_host(host.stable_id())
                .unwrap()
                .active,
            None
        );
        assert!(
            !adapter
                .dispatch(&mut context, document, &primary_tab)
                .unwrap()
                .prevent_default
        );
    }

    #[test]
    fn pointer_on_dock_handle_changes_split_ratio() {
        let mut context = AppContext::new();
        let document = DocumentId::new(1).unwrap();
        let first = context
            .create_component(document, Text::new("first"))
            .unwrap()
            .stable_id();
        let second = context
            .create_component(document, Text::new("second"))
            .unwrap()
            .stable_id();
        let dock = context
            .create_component(
                document,
                Dock::new(DockNode::split(
                    DockAxis::Horizontal,
                    0.4,
                    DockNode::item("inspector", Some(first)),
                    DockNode::item("console", Some(second)),
                )),
            )
            .unwrap();
        context.assemble_dock(dock).unwrap();
        let handle = context.world().node(dock.stable_id()).unwrap().children[1];
        let mut layout = MutationQueue::new();
        layout.write_layout(
            dock.stable_id(),
            LayoutBox {
                x: 0.0,
                y: 0.0,
                width: 400.0,
                height: 200.0,
            },
        );
        layout.write_layout(
            handle,
            LayoutBox {
                x: 156.8,
                y: 0.0,
                width: 8.0,
                height: 200.0,
            },
        );
        context.commit_mutations(layout).unwrap();
        context.rebuild_hit_test(document);

        let mut adapter = RuntimeInputAdapter::default();
        assert!(
            adapter
                .dispatch(
                    &mut context,
                    document,
                    &pointer(PointerPhase::Down, 160.0, 20.0)
                )
                .unwrap()
                .prevent_default
        );
        assert!(
            adapter
                .dispatch(
                    &mut context,
                    document,
                    &pointer(PointerPhase::Move, 200.0, 20.0)
                )
                .unwrap()
                .prevent_default
        );
        assert!(
            adapter
                .dispatch(
                    &mut context,
                    document,
                    &pointer(PointerPhase::Up, 200.0, 20.0)
                )
                .unwrap()
                .prevent_default
        );
        let ratio = context
            .read(dock, |dock| match &dock.root {
                DockNode::Split { ratio, .. } => *ratio,
                _ => panic!("split"),
            })
            .unwrap();
        assert!((ratio - (0.4_f32 + 40.0 / 392.0).clamp(0.05, 0.95)).abs() < 0.001);
    }

    #[test]
    fn keyboard_arrow_right_on_focused_dock_handle_changes_ratio() {
        let mut context = AppContext::new();
        let document = DocumentId::new(1).unwrap();
        let first = context
            .create_component(document, Text::new("first"))
            .unwrap()
            .stable_id();
        let second = context
            .create_component(document, Text::new("second"))
            .unwrap()
            .stable_id();
        let dock = context
            .create_component(
                document,
                Dock::new(DockNode::split(
                    DockAxis::Horizontal,
                    0.4,
                    DockNode::item("inspector", Some(first)),
                    DockNode::item("console", Some(second)),
                )),
            )
            .unwrap();
        context.assemble_dock(dock).unwrap();
        let handle = context.world().node(dock.stable_id()).unwrap().children[1];
        assert!(context.focus_node(document, handle).unwrap());

        let event = InputEvent::Keyboard {
            pressed: true,
            key: "ArrowRight".into(),
            text: None,
            code: "ArrowRight".into(),
            repeat: false,
            modifiers: InputModifiers::default(),
        };
        assert!(
            RuntimeInputAdapter::default()
                .dispatch(&mut context, document, &event)
                .unwrap()
                .prevent_default
        );
        let ratio = context
            .read(dock, |dock| match &dock.root {
                DockNode::Split { ratio, .. } => *ratio,
                _ => panic!("split"),
            })
            .unwrap();
        assert!((ratio - 0.45).abs() < 0.001);
    }

    #[test]
    #[cfg(feature = "calendar")]
    fn pointer_on_calendar_heatmap_sets_active_cell() {
        let mut context = AppContext::new();
        let document = DocumentId::new(1).unwrap();
        let heatmap = context
            .create_component(
                document,
                CalendarHeatmap::new([
                    CalendarHeatmapDatum::<()>::new("2026-06-01", 2.0),
                    CalendarHeatmapDatum::<()>::new("2026-06-03", 8.0),
                ]),
            )
            .unwrap();
        let model = context.read(heatmap, CalendarHeatmap::model).unwrap();
        let cell = model
            .cells
            .iter()
            .find(|cell| cell.date == "2026-06-03")
            .expect("June 3");
        context
            .commit_mutations({
                let mut mutations = MutationQueue::new();
                mutations.write_layout(
                    heatmap.stable_id(),
                    LayoutBox {
                        x: 0.0,
                        y: 0.0,
                        width: model.width,
                        height: model.height,
                    },
                );
                mutations
            })
            .unwrap();
        context.rebuild_hit_test(document);

        assert!(
            RuntimeInputAdapter::default()
                .dispatch(
                    &mut context,
                    document,
                    &pointer(PointerPhase::Move, cell.x + 1.0, cell.y + 1.0)
                )
                .unwrap()
                .prevent_default
        );
        assert_eq!(
            context.read(heatmap, |calendar| calendar.active).unwrap(),
            Some(
                model
                    .cells
                    .iter()
                    .position(|item| item.date == "2026-06-03")
                    .expect("index")
            )
        );
        assert!(
            RuntimeInputAdapter::default()
                .dispatch(
                    &mut context,
                    document,
                    &pointer(PointerPhase::Move, 400.0, 400.0)
                )
                .unwrap()
                .prevent_default
        );
        assert!(
            context
                .read(heatmap, |calendar| calendar.active)
                .unwrap()
                .is_none()
        );
    }

    fn edit_key(key: &str, text: Option<&str>, modifiers: InputModifiers) -> InputEvent {
        InputEvent::Keyboard {
            pressed: true,
            key: key.into(),
            text: text.map(str::to_string),
            code: key.into(),
            repeat: false,
            modifiers,
        }
    }

    fn plain_key(key: &str) -> InputEvent {
        edit_key(key, None, InputModifiers::default())
    }

    fn shift_key(key: &str) -> InputEvent {
        edit_key(
            key,
            None,
            InputModifiers {
                shift: true,
                ..InputModifiers::default()
            },
        )
    }

    fn meta_key(key: &str) -> InputEvent {
        edit_key(
            key,
            None,
            InputModifiers {
                meta: true,
                ..InputModifiers::default()
            },
        )
    }

    fn textarea_selection(context: &AppContext, node: StableNodeId) -> (String, usize, usize) {
        let state = context.world().text_input(node).unwrap();
        (
            state.value.to_owned(),
            state.selection.anchor,
            state.selection.focus,
        )
    }

    #[test]
    fn a_focused_number_input_edits_its_draft_and_keeps_its_stepper_keys() {
        let mut context = AppContext::new();
        let document = DocumentId::new(1).unwrap();
        let input = context
            .create_component(
                document,
                nana_ui_runtime::NumberInput::new(1.0).range(0.0, 100.0),
            )
            .unwrap();
        let node = input.stable_id();
        assert!(context.focus_node(document, node).unwrap());
        let mut adapter = RuntimeInputAdapter::default();
        let value = |context: &AppContext| {
            context
                .read(input, nana_ui_runtime::NumberInput::value)
                .unwrap()
        };
        let control = |key: &str, shift: bool| {
            edit_key(
                key,
                None,
                InputModifiers {
                    control: true,
                    shift,
                    ..InputModifiers::default()
                },
            )
        };

        // ArrowUp steps the value; it is not a caret move.
        assert!(
            adapter
                .dispatch(&mut context, document, &plain_key("ArrowUp"))
                .unwrap()
                .prevent_default
        );
        assert_eq!(value(&context), 2.0);
        assert_eq!(textarea_selection(&context, node), ("2".into(), 1, 1));

        // Left moves the caret inside the draft and typing lands there.
        adapter
            .dispatch(&mut context, document, &plain_key("ArrowLeft"))
            .unwrap();
        assert_eq!(textarea_selection(&context, node), ("2".into(), 0, 0));
        adapter
            .dispatch(
                &mut context,
                document,
                &edit_key("1", Some("1"), InputModifiers::default()),
            )
            .unwrap();
        assert_eq!(textarea_selection(&context, node), ("12".into(), 1, 1));

        // Shift+ArrowUp selects to the start like any single-line field.
        assert!(
            adapter
                .dispatch(&mut context, document, &shift_key("ArrowUp"))
                .unwrap()
                .prevent_default
        );
        assert_eq!(textarea_selection(&context, node), ("12".into(), 1, 0));
        assert_eq!(value(&context), 2.0, "a selecting key does not step");
        adapter
            .dispatch(&mut context, document, &plain_key("ArrowRight"))
            .unwrap();

        // Ctrl+Z / Ctrl+Shift+Z walk the draft's history.
        adapter
            .dispatch(&mut context, document, &control("z", false))
            .unwrap();
        assert_eq!(textarea_selection(&context, node).0, "2");
        adapter
            .dispatch(&mut context, document, &control("z", true))
            .unwrap();
        assert_eq!(textarea_selection(&context, node).0, "12");
        assert_eq!(value(&context), 2.0, "typing committed no number to redo");

        // Enter commits a pending draft instead of submitting a text field.
        adapter
            .dispatch(
                &mut context,
                document,
                &edit_key("Delete", None, InputModifiers::default()),
            )
            .unwrap();
        assert_eq!(textarea_selection(&context, node).0, "1");
        assert!(
            adapter
                .dispatch(&mut context, document, &plain_key("Enter"))
                .unwrap()
                .prevent_default
        );
        assert_eq!(value(&context), 1.0);

        // At its bound the field still owns ArrowUp: it does not fall through
        // to routing that could carry focus out of it. The same holds while
        // an IME composition hides the editor.
        context.set_number_value(input, 100.0).unwrap();
        for composing in [false, true] {
            if composing {
                context
                    .set_ime_preedit(document, "ｘ".into(), None)
                    .unwrap();
            }
            assert!(
                adapter
                    .dispatch(&mut context, document, &plain_key("ArrowUp"))
                    .unwrap()
                    .prevent_default,
                "composing: {composing}"
            );
            assert_eq!(value(&context), 100.0);
            assert_eq!(context.world().focused(document), Some(node));
        }
        context.clear_ime(document).unwrap();

        // Alt+Enter commits like Enter; it is not swallowed as a text
        // field's submit.
        adapter
            .dispatch(
                &mut context,
                document,
                &edit_key("0", Some("0"), InputModifiers::default()),
            )
            .unwrap();
        adapter
            .dispatch(
                &mut context,
                document,
                &edit_key(
                    "Enter",
                    None,
                    InputModifiers {
                        alt: true,
                        ..InputModifiers::default()
                    },
                ),
            )
            .unwrap();
        assert_eq!(value(&context), 100.0, "1000 clamps to the maximum");
        assert_eq!(textarea_selection(&context, node).0, "100");

        // An Enter with nothing to commit is not swallowed: a dialog or form
        // around the field can still confirm on it. The control text hosts
        // report with Enter and Escape never lands in the draft.
        for (key, text) in [("Enter", "\r"), ("Escape", "\u{1b}")] {
            assert!(
                !adapter
                    .dispatch(
                        &mut context,
                        document,
                        &edit_key(key, Some(text), InputModifiers::default()),
                    )
                    .unwrap()
                    .prevent_default,
                "{key}"
            );
            assert_eq!(textarea_selection(&context, node).0, "100", "{key}");
        }
    }

    #[test]
    fn a_composing_text_input_keeps_its_navigation_keys() {
        let mut context = AppContext::new();
        let document = DocumentId::new(1).unwrap();
        let input = context
            .create_component(document, TextInput::new("abc"))
            .unwrap();
        let node = input.stable_id();
        assert!(context.focus_node(document, node).unwrap());
        context
            .set_ime_preedit(document, "に".into(), None)
            .unwrap();
        let mut adapter = RuntimeInputAdapter::default();
        for key in ["ArrowUp", "ArrowDown", "ArrowLeft", "Home"] {
            assert!(
                adapter
                    .dispatch(&mut context, document, &plain_key(key))
                    .unwrap()
                    .prevent_default,
                "{key}"
            );
            assert_eq!(context.world().focused(document), Some(node));
            assert_eq!(textarea_selection(&context, node), ("abc".into(), 3, 3));
        }
    }

    #[test]
    fn a_composing_search_dropdown_keeps_its_list_navigation() {
        use nana_ui_runtime::{SearchDropdown, SearchDropdownOption};
        let mut context = AppContext::new();
        let document = DocumentId::new(1).unwrap();
        let search = context
            .create_component(
                document,
                SearchDropdown::new(None::<&str>)
                    .options([
                        SearchDropdownOption::new("a", "Alpha"),
                        SearchDropdownOption::new("b", "Beta"),
                    ])
                    .opened(true),
            )
            .unwrap();
        assert!(context.focus_node(document, search.stable_id()).unwrap());
        context
            .set_ime_preedit(document, "に".into(), None)
            .unwrap();
        let before = context.read(search, |field| field.highlighted).unwrap();
        RuntimeInputAdapter::default()
            .dispatch(&mut context, document, &plain_key("ArrowDown"))
            .unwrap();
        assert_ne!(
            context.read(search, |field| field.highlighted).unwrap(),
            before,
            "a composite surface is not a plain editor: its list still moves"
        );
    }

    #[test]
    fn arrow_keys_move_and_extend_the_focused_textarea_caret() {
        let mut context = AppContext::new();
        let document = DocumentId::new(1).unwrap();
        let area = context
            .create_component(document, TextArea::new("abcdef"))
            .unwrap();
        let node = area.stable_id();
        assert!(context.focus_node(document, node).unwrap());
        let mut adapter = RuntimeInputAdapter::default();

        // The caret starts at the value end; Left steps back one grapheme.
        assert!(
            adapter
                .dispatch(&mut context, document, &plain_key("ArrowLeft"))
                .unwrap()
                .prevent_default
        );
        assert_eq!(textarea_selection(&context, node), ("abcdef".into(), 5, 5));
        assert!(
            adapter
                .dispatch(&mut context, document, &plain_key("Home"))
                .unwrap()
                .prevent_default
        );
        assert_eq!(textarea_selection(&context, node), ("abcdef".into(), 0, 0));
        assert!(
            adapter
                .dispatch(&mut context, document, &plain_key("End"))
                .unwrap()
                .prevent_default
        );
        assert_eq!(textarea_selection(&context, node), ("abcdef".into(), 6, 6));

        // Shift+Left extends the selection; typing replaces it.
        assert!(
            adapter
                .dispatch(&mut context, document, &shift_key("ArrowLeft"))
                .unwrap()
                .prevent_default
        );
        assert!(
            adapter
                .dispatch(&mut context, document, &shift_key("ArrowLeft"))
                .unwrap()
                .prevent_default
        );
        assert_eq!(textarea_selection(&context, node), ("abcdef".into(), 6, 4));
        assert!(
            adapter
                .dispatch(
                    &mut context,
                    document,
                    &edit_key("x", Some("X"), InputModifiers::default())
                )
                .unwrap()
                .prevent_default
        );
        assert_eq!(textarea_selection(&context, node), ("abcdX".into(), 5, 5));
    }

    #[test]
    fn vertical_arrows_fall_back_to_logical_lines_without_a_shaper() {
        let mut context = AppContext::new();
        let document = DocumentId::new(1).unwrap();
        let area = context
            .create_component(document, TextArea::new("abc\ndefg\nhi"))
            .unwrap();
        let node = area.stable_id();
        assert!(context.focus_node(document, node).unwrap());
        let mut adapter = RuntimeInputAdapter::default();

        assert!(
            adapter
                .dispatch(&mut context, document, &plain_key("ArrowLeft"))
                .unwrap()
                .prevent_default
        );
        assert!(
            adapter
                .dispatch(&mut context, document, &plain_key("ArrowLeft"))
                .unwrap()
                .prevent_default
        );
        assert_eq!(
            textarea_selection(&context, node),
            ("abc\ndefg\nhi".into(), 9, 9)
        );

        // Up keeps the grapheme column: column 0 lands on the line start.
        assert!(
            adapter
                .dispatch(&mut context, document, &plain_key("ArrowUp"))
                .unwrap()
                .prevent_default
        );
        assert_eq!(
            textarea_selection(&context, node),
            ("abc\ndefg\nhi".into(), 4, 4)
        );
        assert!(
            adapter
                .dispatch(&mut context, document, &plain_key("ArrowDown"))
                .unwrap()
                .prevent_default
        );
        assert_eq!(
            textarea_selection(&context, node),
            ("abc\ndefg\nhi".into(), 9, 9)
        );

        // Cmd+Up / Cmd+Down jump to the document edges.
        assert!(
            adapter
                .dispatch(&mut context, document, &meta_key("ArrowUp"))
                .unwrap()
                .prevent_default
        );
        assert_eq!(
            textarea_selection(&context, node),
            ("abc\ndefg\nhi".into(), 0, 0)
        );
        assert!(
            adapter
                .dispatch(&mut context, document, &meta_key("ArrowDown"))
                .unwrap()
                .prevent_default
        );
        assert_eq!(
            textarea_selection(&context, node),
            ("abc\ndefg\nhi".into(), 11, 11)
        );
    }

    #[test]
    fn delete_keys_remove_selections_words_and_line_spans() {
        let mut context = AppContext::new();
        let document = DocumentId::new(1).unwrap();
        let area = context
            .create_component(document, TextArea::new("one two three"))
            .unwrap();
        let node = area.stable_id();
        assert!(context.focus_node(document, node).unwrap());
        let mut adapter = RuntimeInputAdapter::default();

        // Forward delete at the caret end is a no-op that still stays owned.
        let word_modifier = InputModifiers {
            alt: true,
            ..InputModifiers::default()
        };
        assert!(
            adapter
                .dispatch(
                    &mut context,
                    document,
                    &edit_key("Backspace", None, word_modifier)
                )
                .unwrap()
                .prevent_default
        );
        assert_eq!(textarea_selection(&context, node).0, "one two ");

        let meta = InputModifiers {
            meta: true,
            ..InputModifiers::default()
        };
        assert!(
            adapter
                .dispatch(&mut context, document, &edit_key("Backspace", None, meta))
                .unwrap()
                .prevent_default
        );
        assert_eq!(textarea_selection(&context, node).0, "");

        // Forward delete at the value end declines; typing still works.
        assert!(
            !adapter
                .dispatch(&mut context, document, &plain_key("Delete"))
                .unwrap()
                .prevent_default
        );
        assert!(
            adapter
                .dispatch(
                    &mut context,
                    document,
                    &edit_key("h", Some("h"), InputModifiers::default())
                )
                .unwrap()
                .prevent_default
        );
        assert_eq!(textarea_selection(&context, node).0, "h");
    }

    #[test]
    fn code_editor_newline_copies_indent_and_completes_pairs() {
        let mut context = AppContext::new();
        let document = DocumentId::new(1).unwrap();
        let area = context
            .create_component(document, TextArea::new("fn a() {\n  x").code_editor(true))
            .unwrap();
        let node = area.stable_id();
        assert!(context.focus_node(document, node).unwrap());
        let mut adapter = RuntimeInputAdapter::default();

        // Enter after indented content copies the indentation.
        assert!(
            adapter
                .dispatch(&mut context, document, &plain_key("Enter"))
                .unwrap()
                .prevent_default
        );
        assert_eq!(
            textarea_selection(&context, node),
            ("fn a() {\n  x\n  ".into(), 15, 15)
        );

        // Typing an open brace completes the pair and parks inside it.
        assert!(
            adapter
                .dispatch(
                    &mut context,
                    document,
                    &edit_key("{", Some("{"), InputModifiers::default())
                )
                .unwrap()
                .prevent_default
        );
        assert_eq!(
            textarea_selection(&context, node),
            ("fn a() {\n  x\n  {}".into(), 16, 16)
        );

        // Enter between the pair opens a middle line at the deeper level.
        assert!(
            adapter
                .dispatch(&mut context, document, &plain_key("Enter"))
                .unwrap()
                .prevent_default
        );
        assert_eq!(
            textarea_selection(&context, node).0,
            "fn a() {\n  x\n  {\n  \t\n  }"
        );
        assert_eq!(textarea_selection(&context, node).2, 20);
    }

    #[test]
    fn code_editor_comment_toggle_and_tab_indent_the_line() {
        let mut context = AppContext::new();
        let document = DocumentId::new(1).unwrap();
        let area = context
            .create_component(document, TextArea::new("  x").code_editor(true))
            .unwrap();
        let node = area.stable_id();
        assert!(context.focus_node(document, node).unwrap());
        let mut adapter = RuntimeInputAdapter::default();
        let meta = InputModifiers {
            meta: true,
            ..InputModifiers::default()
        };

        assert!(
            adapter
                .dispatch(&mut context, document, &edit_key("/", None, meta))
                .unwrap()
                .prevent_default
        );
        assert_eq!(textarea_selection(&context, node).0, "  //x");
        assert!(
            adapter
                .dispatch(&mut context, document, &edit_key("/", None, meta))
                .unwrap()
                .prevent_default
        );
        assert_eq!(textarea_selection(&context, node).0, "  x");

        // Tab indents the caret line; Shift+Tab outdents again.
        assert!(
            adapter
                .dispatch(&mut context, document, &plain_key("Home"))
                .unwrap()
                .prevent_default
        );
        assert!(
            adapter
                .dispatch(&mut context, document, &plain_key("Tab"))
                .unwrap()
                .prevent_default
        );
        assert_eq!(textarea_selection(&context, node).0, "\t  x");
        assert!(
            adapter
                .dispatch(&mut context, document, &shift_key("Tab"))
                .unwrap()
                .prevent_default
        );
        assert_eq!(textarea_selection(&context, node).0, "  x");
    }

    #[test]
    fn plain_textarea_enter_inserts_a_bare_newline_without_pairing() {
        let mut context = AppContext::new();
        let document = DocumentId::new(1).unwrap();
        let area = context
            .create_component(document, TextArea::new("ab"))
            .unwrap();
        let node = area.stable_id();
        assert!(context.focus_node(document, node).unwrap());
        let mut adapter = RuntimeInputAdapter::default();

        assert!(
            adapter
                .dispatch(&mut context, document, &plain_key("Enter"))
                .unwrap()
                .prevent_default
        );
        assert_eq!(textarea_selection(&context, node).0, "ab\n");
        assert!(
            adapter
                .dispatch(
                    &mut context,
                    document,
                    &edit_key("(", Some("("), InputModifiers::default())
                )
                .unwrap()
                .prevent_default
        );
        assert_eq!(textarea_selection(&context, node).0, "ab\n(");
    }

    #[test]
    fn pointer_press_places_the_caret_and_multi_click_selects() {
        let mut context = AppContext::new();
        let document = DocumentId::new(1).unwrap();
        let input = context
            .create_component(document, TextInput::new("hello world"))
            .unwrap();
        let node = input.stable_id();
        let mut layout = MutationQueue::new();
        layout.write_layout(
            node,
            LayoutBox {
                x: 0.0,
                y: 0.0,
                width: 200.0,
                height: 32.0,
            },
        );
        layout.set_standard_visual(
            node,
            Some(nana_ui_runtime::StandardVisual::TextInput {
                placeholder: std::sync::Arc::from(""),
                size: nana_ui_core::ControlSize::Medium,
                secure: false,
                invalid: false,
                steppers: false,
                diagnostics: std::sync::Arc::from([]),
                matches: std::sync::Arc::from([]),
                color_swatches: std::sync::Arc::from([]),
                atoms: Arc::from([]),
                line_numbers: false,
                indent_guides: None,
                folds: std::sync::Arc::from([]),
                git_marks: std::sync::Arc::from([]),
                editor_options: Default::default(),
            }),
        );
        context.commit_mutations(layout).unwrap();
        context.take_system_work();
        context.rebuild_hit_test(document);

        let mut shaper = MeasureTextShaper;
        let mut adapter = RuntimeInputAdapter::default();
        let click = |x: f32, y: f32, phase: PointerPhase| InputEvent::Pointer {
            phase,
            pointer_id: 7,
            pointer_type: PointerType::Mouse,
            x,
            y,
            screen_x: x,
            screen_y: y,
            button: 0,
            buttons: u16::from(phase == PointerPhase::Down || phase == PointerPhase::Move),
            pressure: 1.0,
            tangential_pressure: 0.0,
            tilt_x: 0,
            tilt_y: 0,
            twist: 0,
            is_primary: true,
            activation_click: false,
            modifiers: InputModifiers::default(),
        };

        // A press past the line end parks the caret on the line end.
        assert!(
            adapter
                .dispatch_with_shaper(
                    &mut context,
                    document,
                    &click(190.0, 16.0, PointerPhase::Down),
                    Duration::from_millis(1_000),
                    Some(&mut shaper),
                )
                .unwrap()
                .prevent_default
        );
        adapter
            .dispatch_with_shaper(
                &mut context,
                document,
                &click(190.0, 16.0, PointerPhase::Up),
                Duration::from_millis(1_020),
                Some(&mut shaper),
            )
            .unwrap();
        assert_eq!(
            textarea_selection(&context, node),
            ("hello world".into(), 11, 11)
        );

        // A quick second press selects the word under the caret.
        assert!(
            adapter
                .dispatch_with_shaper(
                    &mut context,
                    document,
                    &click(190.0, 16.0, PointerPhase::Down),
                    Duration::from_millis(1_060),
                    Some(&mut shaper),
                )
                .unwrap()
                .prevent_default
        );
        assert_eq!(
            textarea_selection(&context, node),
            ("hello world".into(), 6, 11)
        );
        adapter
            .dispatch_with_shaper(
                &mut context,
                document,
                &click(190.0, 16.0, PointerPhase::Up),
                Duration::from_millis(1_080),
                Some(&mut shaper),
            )
            .unwrap();
    }

    #[test]
    fn pointer_drag_extends_the_selection_from_the_press_anchor() {
        let mut context = AppContext::new();
        let document = DocumentId::new(1).unwrap();
        let input = context
            .create_component(document, TextInput::new("hello world"))
            .unwrap();
        let node = input.stable_id();
        let mut layout = MutationQueue::new();
        layout.write_layout(
            node,
            LayoutBox {
                x: 0.0,
                y: 0.0,
                width: 200.0,
                height: 32.0,
            },
        );
        layout.set_standard_visual(
            node,
            Some(nana_ui_runtime::StandardVisual::TextInput {
                placeholder: std::sync::Arc::from(""),
                size: nana_ui_core::ControlSize::Medium,
                secure: false,
                invalid: false,
                steppers: false,
                diagnostics: std::sync::Arc::from([]),
                matches: std::sync::Arc::from([]),
                color_swatches: std::sync::Arc::from([]),
                atoms: Arc::from([]),
                line_numbers: false,
                indent_guides: None,
                folds: std::sync::Arc::from([]),
                git_marks: std::sync::Arc::from([]),
                editor_options: Default::default(),
            }),
        );
        context.commit_mutations(layout).unwrap();
        context.take_system_work();
        context.rebuild_hit_test(document);

        let mut shaper = MeasureTextShaper;
        let mut adapter = RuntimeInputAdapter::default();
        let pointer_event = |phase: PointerPhase, x: f32| InputEvent::Pointer {
            phase,
            pointer_id: 3,
            pointer_type: PointerType::Mouse,
            x,
            y: 16.0,
            screen_x: x,
            screen_y: 16.0,
            button: 0,
            buttons: u16::from(phase != PointerPhase::Up),
            pressure: 1.0,
            tangential_pressure: 0.0,
            tilt_x: 0,
            tilt_y: 0,
            twist: 0,
            is_primary: true,
            activation_click: false,
            modifiers: InputModifiers::default(),
        };

        adapter
            .dispatch_with_shaper(
                &mut context,
                document,
                &pointer_event(PointerPhase::Down, 190.0),
                Duration::from_millis(2_000),
                Some(&mut shaper),
            )
            .unwrap();
        assert!(
            adapter
                .dispatch_with_shaper(
                    &mut context,
                    document,
                    &pointer_event(PointerPhase::Move, 0.0),
                    Duration::from_millis(2_010),
                    Some(&mut shaper),
                )
                .unwrap()
                .prevent_default
        );
        assert_eq!(
            textarea_selection(&context, node),
            ("hello world".into(), 11, 0)
        );
        adapter
            .dispatch_with_shaper(
                &mut context,
                document,
                &pointer_event(PointerPhase::Up, 0.0),
                Duration::from_millis(2_020),
                Some(&mut shaper),
            )
            .unwrap();
    }

    /// 挂一个收集 TextChanged 的观察者，供查找/替换命令断言事件发射。
    fn track_text_changed(
        context: &mut AppContext,
        area: Entity<TextArea>,
    ) -> Arc<Mutex<Vec<String>>> {
        let events = Arc::new(Mutex::new(Vec::new()));
        let sink = Arc::clone(&events);
        context
            .on(area, move |_area, event: &TextChanged, _cx| {
                sink.lock().unwrap().push(event.value.to_string());
            })
            .unwrap();
        events
    }

    /// 多行编辑器拖拽移动测试的公共装配：两行 `abc\ndef`，字符宽 10、
    /// 行高 12、零内边距（offset = 列×10 + 行×12 命中）。返回
    /// `(context, document, node, 事件收集器)`。
    fn drag_drop_editor() -> (
        AppContext,
        DocumentId,
        StableNodeId,
        Arc<Mutex<Vec<String>>>,
    ) {
        let mut context = AppContext::new();
        let document = DocumentId::new(1).unwrap();
        let area = context
            .create_component(
                document,
                TextArea::new("abc\ndef").style(nana_ui_runtime::NodeStyle {
                    layout: std::sync::Arc::new(nana_ui_core::LayoutStyle {
                        padding: Some(nana_ui_core::LengthSpec::Px(0.0)),
                        font_size: Some(10.0),
                        line_height: Some(nana_ui_core::LineHeightSpec::Absolute(12.0)),
                        min_height: None,
                        ..nana_ui_core::LayoutStyle::default()
                    }),
                    ..nana_ui_runtime::NodeStyle::default()
                }),
            )
            .unwrap();
        let node = area.stable_id();
        let mut layout = MutationQueue::new();
        layout.write_layout(
            node,
            LayoutBox {
                x: 0.0,
                y: 0.0,
                width: 200.0,
                height: 64.0,
            },
        );
        context.commit_mutations(layout).unwrap();
        context.take_system_work();
        context.rebuild_hit_test(document);
        assert!(context.focus_node(document, node).unwrap());
        let events = track_text_changed(&mut context, area);
        (context, document, node, events)
    }

    /// 带 x/y 与修饰键的指针事件构造器。
    fn drag_pointer_event(
        phase: PointerPhase,
        x: f32,
        y: f32,
        modifiers: InputModifiers,
    ) -> InputEvent {
        InputEvent::Pointer {
            phase,
            pointer_id: 3,
            pointer_type: PointerType::Mouse,
            x,
            y,
            screen_x: x,
            screen_y: y,
            button: 0,
            buttons: u16::from(phase != PointerPhase::Up),
            pressure: 1.0,
            tangential_pressure: 0.0,
            tilt_x: 0,
            tilt_y: 0,
            twist: 0,
            is_primary: true,
            activation_click: false,
            modifiers,
        }
    }

    /// 选中 `def`：End 到文档尾，Shift+Left 三次（偏移 7..4）。
    fn select_trailing_def(
        adapter: &mut RuntimeInputAdapter,
        context: &mut AppContext,
        document: DocumentId,
        shaper: &mut MeasureTextShaper,
    ) {
        adapter
            .dispatch_with_shaper(
                context,
                document,
                &plain_key("End"),
                Duration::from_millis(1_000),
                Some(shaper),
            )
            .unwrap();
        for _ in 0..3 {
            adapter
                .dispatch_with_shaper(
                    context,
                    document,
                    &shift_key("ArrowLeft"),
                    Duration::from_millis(1_000),
                    Some(shaper),
                )
                .unwrap();
        }
    }

    /// 无修饰键的拖拽指针事件（按下/移动/释放）。
    fn pointer_down(x: f32, y: f32) -> InputEvent {
        drag_pointer_event(PointerPhase::Down, x, y, InputModifiers::default())
    }

    fn pointer_move(x: f32, y: f32) -> InputEvent {
        drag_pointer_event(PointerPhase::Move, x, y, InputModifiers::default())
    }

    fn pointer_up(x: f32, y: f32) -> InputEvent {
        drag_pointer_event(PointerPhase::Up, x, y, InputModifiers::default())
    }

    /// 拖拽移动主流程：选中 `def`（第二行 0..3 列 → 偏移 4..7），在选区
    /// 内按下并拖到第一行行首释放 = 移动文本，选区落在插入文本上，整
    /// 个移动只发一次变更（单步撤销的修订语义）。
    #[test]
    fn drag_selection_moves_text_in_one_revision() {
        let (mut context, document, node, events) = drag_drop_editor();
        let mut shaper = MeasureTextShaper;
        let mut adapter = RuntimeInputAdapter::default();
        // 选中 "def"：End 到文档尾，Shift+Left 三次。
        select_trailing_def(&mut adapter, &mut context, document, &mut shaper);
        assert_eq!(
            textarea_selection(&context, node),
            ("abc\ndef".into(), 7, 4)
        );

        // 选区内按下（"e"，offset 5）→ 不塌缩选区。
        adapter
            .dispatch_with_shaper(
                &mut context,
                document,
                &pointer_down(15.0, 18.0),
                Duration::from_millis(2_000),
                Some(&mut shaper),
            )
            .unwrap();
        assert_eq!(
            textarea_selection(&context, node),
            ("abc\ndef".into(), 7, 4)
        );
        // 超过阈值拖到第一行行首（offset 0）。
        adapter
            .dispatch_with_shaper(
                &mut context,
                document,
                &pointer_move(0.0, 6.0),
                Duration::from_millis(2_010),
                Some(&mut shaper),
            )
            .unwrap();
        adapter
            .dispatch_with_shaper(
                &mut context,
                document,
                &pointer_up(0.0, 6.0),
                Duration::from_millis(2_020),
                Some(&mut shaper),
            )
            .unwrap();
        // "def" 移到文档头，选区落在插入文本上；单次变更（一步撤销）。
        assert_eq!(
            textarea_selection(&context, node),
            ("defabc\n".into(), 0, 3)
        );
        assert_eq!(*events.lock().unwrap(), vec!["defabc\n".to_owned()]);
    }

    /// 落点在源选区边界（target == start / target == end）是退化 no-op：
    /// 文本与选区都保持原状，不产生变更事件。
    #[test]
    fn drag_to_selection_boundary_is_a_no_op() {
        let (mut context, document, node, events) = drag_drop_editor();
        let mut shaper = MeasureTextShaper;
        let mut adapter = RuntimeInputAdapter::default();
        select_trailing_def(&mut adapter, &mut context, document, &mut shaper);
        // target == start（offset 4）：选区头。
        adapter
            .dispatch_with_shaper(
                &mut context,
                document,
                &pointer_down(15.0, 18.0),
                Duration::from_millis(2_000),
                Some(&mut shaper),
            )
            .unwrap();
        adapter
            .dispatch_with_shaper(
                &mut context,
                document,
                &pointer_move(2.0, 18.0),
                Duration::from_millis(2_010),
                Some(&mut shaper),
            )
            .unwrap();
        adapter
            .dispatch_with_shaper(
                &mut context,
                document,
                &pointer_up(2.0, 18.0),
                Duration::from_millis(2_020),
                Some(&mut shaper),
            )
            .unwrap();
        assert_eq!(
            textarea_selection(&context, node),
            ("abc\ndef".into(), 7, 4)
        );
        // target == end（offset 7）：选区尾。
        adapter
            .dispatch_with_shaper(
                &mut context,
                document,
                &pointer_down(15.0, 18.0),
                Duration::from_millis(3_000),
                Some(&mut shaper),
            )
            .unwrap();
        adapter
            .dispatch_with_shaper(
                &mut context,
                document,
                &pointer_move(35.0, 18.0),
                Duration::from_millis(3_010),
                Some(&mut shaper),
            )
            .unwrap();
        adapter
            .dispatch_with_shaper(
                &mut context,
                document,
                &pointer_up(35.0, 18.0),
                Duration::from_millis(3_020),
                Some(&mut shaper),
            )
            .unwrap();
        assert_eq!(
            textarea_selection(&context, node),
            ("abc\ndef".into(), 7, 4)
        );
        assert!(events.lock().unwrap().is_empty());
    }

    /// Alt 拖拽 = 复制：原文本保留，选区落在插入的副本上。
    #[test]
    fn alt_drag_selection_copies_text() {
        let (mut context, document, node, events) = drag_drop_editor();
        let mut shaper = MeasureTextShaper;
        let mut adapter = RuntimeInputAdapter::default();
        select_trailing_def(&mut adapter, &mut context, document, &mut shaper);
        let alt = InputModifiers {
            alt: true,
            ..Default::default()
        };
        adapter
            .dispatch_with_shaper(
                &mut context,
                document,
                &drag_pointer_event(PointerPhase::Down, 15.0, 18.0, alt),
                Duration::from_millis(2_000),
                Some(&mut shaper),
            )
            .unwrap();
        adapter
            .dispatch_with_shaper(
                &mut context,
                document,
                &drag_pointer_event(PointerPhase::Move, 0.0, 6.0, alt),
                Duration::from_millis(2_010),
                Some(&mut shaper),
            )
            .unwrap();
        adapter
            .dispatch_with_shaper(
                &mut context,
                document,
                &drag_pointer_event(PointerPhase::Up, 0.0, 6.0, alt),
                Duration::from_millis(2_020),
                Some(&mut shaper),
            )
            .unwrap();
        assert_eq!(
            textarea_selection(&context, node),
            ("defabc\ndef".into(), 0, 3)
        );
        assert_eq!(*events.lock().unwrap(), vec!["defabc\ndef".to_owned()]);
    }

    /// 低于阈值：按下选区后小位移释放不移动文本，按原点击语义落 caret。
    #[test]
    fn selection_press_below_threshold_falls_back_to_click() {
        let (mut context, document, node, events) = drag_drop_editor();
        let mut shaper = MeasureTextShaper;
        let mut adapter = RuntimeInputAdapter::default();
        select_trailing_def(&mut adapter, &mut context, document, &mut shaper);
        adapter
            .dispatch_with_shaper(
                &mut context,
                document,
                &drag_pointer_event(PointerPhase::Down, 15.0, 18.0, InputModifiers::default()),
                Duration::from_millis(2_000),
                Some(&mut shaper),
            )
            .unwrap();
        // 2px 位移，未过阈值。
        adapter
            .dispatch_with_shaper(
                &mut context,
                document,
                &drag_pointer_event(PointerPhase::Move, 13.0, 18.0, InputModifiers::default()),
                Duration::from_millis(2_010),
                Some(&mut shaper),
            )
            .unwrap();
        adapter
            .dispatch_with_shaper(
                &mut context,
                document,
                &drag_pointer_event(PointerPhase::Up, 13.0, 18.0, InputModifiers::default()),
                Duration::from_millis(2_020),
                Some(&mut shaper),
            )
            .unwrap();
        assert_eq!(
            textarea_selection(&context, node),
            ("abc\ndef".into(), 5, 5)
        );
        assert!(events.lock().unwrap().is_empty());
    }

    /// Esc 取消拖拽：文本与选区保持原状，后续释放不落文本。
    #[test]
    fn escape_cancels_selection_drag() {
        let (mut context, document, node, events) = drag_drop_editor();
        let mut shaper = MeasureTextShaper;
        let mut adapter = RuntimeInputAdapter::default();
        select_trailing_def(&mut adapter, &mut context, document, &mut shaper);
        adapter
            .dispatch_with_shaper(
                &mut context,
                document,
                &drag_pointer_event(PointerPhase::Down, 15.0, 18.0, InputModifiers::default()),
                Duration::from_millis(2_000),
                Some(&mut shaper),
            )
            .unwrap();
        adapter
            .dispatch_with_shaper(
                &mut context,
                document,
                &drag_pointer_event(PointerPhase::Move, 0.0, 6.0, InputModifiers::default()),
                Duration::from_millis(2_010),
                Some(&mut shaper),
            )
            .unwrap();
        assert!(
            adapter
                .dispatch_with_shaper(
                    &mut context,
                    document,
                    &plain_key("Escape"),
                    Duration::from_millis(2_015),
                    Some(&mut shaper),
                )
                .unwrap()
                .prevent_default
        );
        adapter
            .dispatch_with_shaper(
                &mut context,
                document,
                &pointer_up(0.0, 6.0),
                Duration::from_millis(2_020),
                Some(&mut shaper),
            )
            .unwrap();
        assert_eq!(
            textarea_selection(&context, node),
            ("abc\ndef".into(), 7, 4)
        );
        assert!(events.lock().unwrap().is_empty());
    }

    #[test]
    fn find_next_and_previous_select_matches_without_text_changed() {
        let mut context = AppContext::new();
        let document = DocumentId::new(1).unwrap();
        let area = context
            .create_component(document, TextArea::new("ab AB ab"))
            .unwrap();
        let node = area.stable_id();
        assert!(context.focus_node(document, node).unwrap());
        let events = track_text_changed(&mut context, area);
        let sensitive = TextSearchOptions {
            case_sensitive: true,
            ..TextSearchOptions::default()
        };

        // 大小写敏感："ab" 只命中 0..2 与 6..8。
        assert!(
            context
                .find_next_focused_text_match(document, "ab", sensitive, TextFindScope::Document)
                .unwrap()
        );
        assert_eq!(
            textarea_selection(&context, node),
            ("ab AB ab".into(), 0, 2)
        );
        assert!(
            context
                .find_next_focused_text_match(document, "ab", sensitive, TextFindScope::Document)
                .unwrap()
        );
        assert_eq!(
            textarea_selection(&context, node),
            ("ab AB ab".into(), 6, 8)
        );
        // 越过末尾后环绕。
        assert!(
            context
                .find_next_focused_text_match(document, "ab", sensitive, TextFindScope::Document)
                .unwrap()
        );
        assert_eq!(
            textarea_selection(&context, node),
            ("ab AB ab".into(), 0, 2)
        );
        // 大小写不敏感：从当前选区末端起下一个命中是 "AB"。
        assert!(
            context
                .find_next_focused_text_match(
                    document,
                    "ab",
                    TextSearchOptions::default(),
                    TextFindScope::Document
                )
                .unwrap()
        );
        assert_eq!(
            textarea_selection(&context, node),
            ("ab AB ab".into(), 3, 5)
        );
        // 上一个回到第一个 "ab"。
        assert!(
            context
                .find_previous_focused_text_match(
                    document,
                    "ab",
                    sensitive,
                    TextFindScope::Document
                )
                .unwrap()
        );
        assert_eq!(
            textarea_selection(&context, node),
            ("ab AB ab".into(), 0, 2)
        );
        // 纯移动：值不变、不发 TextChanged。
        assert!(events.lock().unwrap().is_empty());
        // 空 query 不命中。
        assert!(
            !context
                .find_next_focused_text_match(document, "", sensitive, TextFindScope::Document)
                .unwrap()
        );
        assert!(
            !context
                .find_previous_focused_text_match(
                    document,
                    "zz",
                    sensitive,
                    TextFindScope::Document
                )
                .unwrap()
        );
    }

    #[test]
    fn replace_focused_text_match_replaces_only_a_matching_selection() {
        let mut context = AppContext::new();
        let document = DocumentId::new(1).unwrap();
        let area = context
            .create_component(document, TextArea::new("ab ab"))
            .unwrap();
        let node = area.stable_id();
        assert!(context.focus_node(document, node).unwrap());
        let events = track_text_changed(&mut context, area);
        let options = TextSearchOptions::default();

        // 选中第一个 "ab" 后替换，并选中替换后的文本。
        context
            .update_component(area, |area, _cx| {
                area.state.selection = TextSelection::new(0, 2);
            })
            .unwrap();
        assert!(
            context
                .replace_focused_text_match(document, "ab", options, "XY", false)
                .unwrap()
        );
        assert_eq!(textarea_selection(&context, node), ("XY ab".into(), 0, 2));
        assert_eq!(*events.lock().unwrap(), vec!["XY ab".to_string()]);

        // 选区不再是匹配（现在是 "XY"），替换拒绝且不发射事件。
        assert!(
            !context
                .replace_focused_text_match(document, "ab", options, "XY", false)
                .unwrap()
        );
        assert_eq!(textarea_selection(&context, node), ("XY ab".into(), 0, 2));

        // 宿主先查找下一个再替换。
        assert!(
            context
                .find_next_focused_text_match(document, "ab", options, TextFindScope::Document)
                .unwrap()
        );
        assert_eq!(textarea_selection(&context, node), ("XY ab".into(), 3, 5));
        assert!(
            context
                .replace_focused_text_match(document, "ab", options, "XY", false)
                .unwrap()
        );
        assert_eq!(textarea_selection(&context, node), ("XY XY".into(), 3, 5));
        assert_eq!(events.lock().unwrap().len(), 2);
    }

    #[test]
    fn replace_all_focused_text_matches_reports_count_and_lands_on_first() {
        let mut context = AppContext::new();
        let document = DocumentId::new(1).unwrap();
        let area = context
            .create_component(document, TextArea::new("ab cd ab"))
            .unwrap();
        let node = area.stable_id();
        assert!(context.focus_node(document, node).unwrap());
        let events = track_text_changed(&mut context, area);

        assert_eq!(
            context
                .replace_all_focused_text_matches(
                    document,
                    "ab",
                    TextSearchOptions {
                        whole_word: true,
                        ..TextSearchOptions::default()
                    },
                    "X",
                    TextFindScope::Document,
                    false,
                )
                .unwrap(),
            2
        );
        assert_eq!(textarea_selection(&context, node), ("X cd X".into(), 0, 1));
        assert_eq!(*events.lock().unwrap(), vec!["X cd X".to_string()]);

        // 没有匹配时不修改、不发射事件、计数为 0。
        assert_eq!(
            context
                .replace_all_focused_text_matches(
                    document,
                    "ab",
                    TextSearchOptions::default(),
                    "X",
                    TextFindScope::Document,
                    false,
                )
                .unwrap(),
            0
        );
        assert_eq!(
            context
                .replace_all_focused_text_matches(
                    document,
                    "",
                    TextSearchOptions::default(),
                    "X",
                    TextFindScope::Document,
                    false
                )
                .unwrap(),
            0
        );
        assert_eq!(events.lock().unwrap().len(), 1);
    }

    #[test]
    fn alt_arrow_keys_move_and_duplicate_the_caret_line() {
        let mut context = AppContext::new();
        let document = DocumentId::new(1).unwrap();
        let area = context
            .create_component(document, TextArea::new("ab\ncd\nef"))
            .unwrap();
        let node = area.stable_id();
        assert!(context.focus_node(document, node).unwrap());
        let events = track_text_changed(&mut context, area);
        let mut adapter = RuntimeInputAdapter::default();
        let alt = InputModifiers {
            alt: true,
            ..InputModifiers::default()
        };
        let alt_shift = InputModifiers {
            alt: true,
            shift: true,
            ..InputModifiers::default()
        };
        // 光标停在 "cd" 行内（偏移 4）。
        context
            .update_component(area, |area, _cx| {
                area.state.selection = TextSelection::caret(4);
            })
            .unwrap();

        // Alt+Up 把 "cd" 移到顶部，选区（光标）跟随移动后的文本。
        assert!(
            adapter
                .dispatch(&mut context, document, &edit_key("ArrowUp", None, alt))
                .unwrap()
                .prevent_default
        );
        assert_eq!(
            textarea_selection(&context, node),
            ("cd\nab\nef".into(), 1, 1)
        );
        assert_eq!(*events.lock().unwrap(), vec!["cd\nab\nef".to_string()]);

        // Alt+Down 移回原位。
        assert!(
            adapter
                .dispatch(&mut context, document, &edit_key("ArrowDown", None, alt))
                .unwrap()
                .prevent_default
        );
        assert_eq!(
            textarea_selection(&context, node),
            ("ab\ncd\nef".into(), 4, 4)
        );

        // Alt+Shift+Down 在下方复制当前行，光标落在副本上。
        assert!(
            adapter
                .dispatch(
                    &mut context,
                    document,
                    &edit_key("ArrowDown", None, alt_shift)
                )
                .unwrap()
                .prevent_default
        );
        assert_eq!(
            textarea_selection(&context, node),
            ("ab\ncd\ncd\nef".into(), 7, 7)
        );

        // 文档边缘：手势仍被消费（不回落为普通光标移动），但没有编辑。
        context
            .update_component(area, |area, _cx| {
                area.state = nana_ui_runtime::TextInputState::new("top\nbottom");
            })
            .unwrap();
        context
            .update_component(area, |area, _cx| {
                area.state.selection = TextSelection::caret(0);
            })
            .unwrap();
        assert!(
            adapter
                .dispatch(&mut context, document, &edit_key("ArrowUp", None, alt))
                .unwrap()
                .prevent_default
        );
        assert_eq!(textarea_selection(&context, node).0, "top\nbottom");
    }

    #[test]
    fn cmd_shift_k_ctrl_j_and_case_keys_transform_the_focused_editor() {
        let mut context = AppContext::new();
        let document = DocumentId::new(1).unwrap();
        let area = context
            .create_component(document, TextArea::new("ab\ncd\nef"))
            .unwrap();
        let node = area.stable_id();
        assert!(context.focus_node(document, node).unwrap());
        let events = track_text_changed(&mut context, area);
        let mut adapter = RuntimeInputAdapter::default();
        let ctrl_shift = InputModifiers {
            control: true,
            shift: true,
            ..InputModifiers::default()
        };
        let ctrl = InputModifiers {
            control: true,
            ..InputModifiers::default()
        };
        // 光标停在 "cd" 行内。
        context
            .update_component(area, |area, _cx| {
                area.state.selection = TextSelection::caret(4);
            })
            .unwrap();

        // Ctrl+Shift+K 删除光标所在行 "cd"。
        assert!(
            adapter
                .dispatch(&mut context, document, &edit_key("k", None, ctrl_shift))
                .unwrap()
                .prevent_default
        );
        assert_eq!(textarea_selection(&context, node), ("ab\nef".into(), 3, 3));
        assert_eq!(*events.lock().unwrap(), vec!["ab\nef".to_string()]);

        // Ctrl+J 合并剩余两行（单空格接缝）。
        context
            .update_component(area, |area, _cx| {
                area.state.selection = TextSelection::caret(1);
            })
            .unwrap();
        assert!(
            adapter
                .dispatch(&mut context, document, &edit_key("j", None, ctrl))
                .unwrap()
                .prevent_default
        );
        assert_eq!(textarea_selection(&context, node).0, "ab ef");
        assert_eq!(events.lock().unwrap().len(), 2);

        // Ctrl+Shift+U 转大写选区，Ctrl+U 转小写。
        context
            .update_component(area, |area, _cx| {
                area.state.selection = TextSelection::new(0, 5);
            })
            .unwrap();
        assert!(
            adapter
                .dispatch(&mut context, document, &edit_key("u", None, ctrl_shift))
                .unwrap()
                .prevent_default
        );
        assert_eq!(textarea_selection(&context, node), ("AB EF".into(), 0, 5));
        assert!(
            adapter
                .dispatch(&mut context, document, &edit_key("u", None, ctrl))
                .unwrap()
                .prevent_default
        );
        assert_eq!(textarea_selection(&context, node), ("ab ef".into(), 0, 5));
        assert_eq!(events.lock().unwrap().len(), 4);
    }

    #[test]
    fn line_transformation_keys_decline_on_single_line_fields() {
        let mut context = AppContext::new();
        let document = DocumentId::new(1).unwrap();
        let input = context
            .create_component(document, TextInput::new("abc"))
            .unwrap();
        let node = input.stable_id();
        assert!(context.focus_node(document, node).unwrap());
        let mut adapter = RuntimeInputAdapter::default();

        // 单行字段没有行块语义：这些键不消费，留给通用路由。
        let ctrl_shift = InputModifiers {
            control: true,
            shift: true,
            ..InputModifiers::default()
        };
        assert!(
            !adapter
                .dispatch(&mut context, document, &edit_key("k", None, ctrl_shift))
                .unwrap()
                .prevent_default
        );
        assert_eq!(textarea_selection(&context, node).0, "abc");
    }

    #[test]
    fn page_keys_page_by_logical_lines_without_a_shaper() {
        let value = (0..40)
            .map(|index| format!("l{index}"))
            .collect::<Vec<_>>()
            .join("\n");
        let mut context = AppContext::new();
        let document = DocumentId::new(1).unwrap();
        let area = context
            .create_component(document, TextArea::new(value.clone()))
            .unwrap();
        let node = area.stable_id();
        assert!(context.focus_node(document, node).unwrap());
        context
            .update_component(area, |area, _cx| {
                area.state.selection = TextSelection::caret(1);
            })
            .unwrap();
        let mut adapter = RuntimeInputAdapter::default();

        // 无 shaper：固定 15 个逻辑行。第 15 行起点在 10 个 3 字节行 + 5 个
        // 4 字节行之后，保持第 1 列。
        assert!(
            adapter
                .dispatch(&mut context, document, &plain_key("PageDown"))
                .unwrap()
                .prevent_default
        );
        let line15 = 10 * 3 + 5 * 4;
        assert_eq!(
            textarea_selection(&context, node),
            (value.clone(), line15 + 1, line15 + 1)
        );
        assert!(
            adapter
                .dispatch(&mut context, document, &plain_key("PageUp"))
                .unwrap()
                .prevent_default
        );
        assert_eq!(textarea_selection(&context, node), (value.clone(), 1, 1));

        // Shift+PageDown 扩展选区（锚点保留在原列）。
        assert!(
            adapter
                .dispatch(&mut context, document, &shift_key("PageDown"))
                .unwrap()
                .prevent_default
        );
        assert_eq!(textarea_selection(&context, node), (value, 1, line15 + 1));
    }

    #[test]
    fn page_keys_with_a_shaper_move_one_viewport_and_clamp() {
        let value = (0..10)
            .map(|index| format!("l{index}"))
            .collect::<Vec<_>>()
            .join("\n");
        let mut context = AppContext::new();
        let document = DocumentId::new(1).unwrap();
        let area = context
            .create_component(document, TextArea::new(value.clone()))
            .unwrap();
        let node = area.stable_id();
        assert!(context.focus_node(document, node).unwrap());
        let mut layout = MutationQueue::new();
        layout.write_layout(
            node,
            LayoutBox {
                x: 0.0,
                y: 0.0,
                width: 200.0,
                height: 300.0,
            },
        );
        context.commit_mutations(layout).unwrap();
        context
            .update_component(area, |area, _cx| {
                area.state.selection = TextSelection::caret(0);
            })
            .unwrap();
        let mut shaper = MeasureTextShaper;
        let mut adapter = RuntimeInputAdapter::default();

        // 视口高于文档：一次 PageDown 钳制到文档末尾，PageUp 回到首行。
        assert!(
            adapter
                .dispatch_with_shaper(
                    &mut context,
                    document,
                    &plain_key("PageDown"),
                    Duration::ZERO,
                    Some(&mut shaper),
                )
                .unwrap()
                .prevent_default
        );
        // 目标列保持 0：落在最后一行行首（而非文档末尾偏移）。
        assert_eq!(textarea_selection(&context, node), (value.clone(), 27, 27));
        assert!(
            adapter
                .dispatch_with_shaper(
                    &mut context,
                    document,
                    &plain_key("PageUp"),
                    Duration::ZERO,
                    Some(&mut shaper),
                )
                .unwrap()
                .prevent_default
        );
        assert_eq!(textarea_selection(&context, node), (value, 0, 0));
    }

    #[test]
    fn goto_focused_text_matching_bracket_jumps_to_the_partner() {
        let mut context = AppContext::new();
        let document = DocumentId::new(1).unwrap();
        let area = context
            .create_component(document, TextArea::new("fn main() {}"))
            .unwrap();
        let node = area.stable_id();
        assert!(context.focus_node(document, node).unwrap());
        let events = track_text_changed(&mut context, area);

        // 光标停在 '{' 之前：跳到配对的 '}' 上（纯移动，不发事件）。
        context
            .update_component(area, |area, _cx| {
                area.state.selection = TextSelection::caret(10);
            })
            .unwrap();
        assert!(
            context
                .goto_focused_text_matching_bracket(document)
                .unwrap()
        );
        assert_eq!(
            textarea_selection(&context, node),
            ("fn main() {}".into(), 11, 11)
        );
        // 再跳一次回到 '{'。
        assert!(
            context
                .goto_focused_text_matching_bracket(document)
                .unwrap()
        );
        assert_eq!(
            textarea_selection(&context, node),
            ("fn main() {}".into(), 10, 10)
        );
        assert!(events.lock().unwrap().is_empty());

        // 邻近没有括号时不消费。
        context
            .update_component(area, |area, _cx| {
                area.state.selection = TextSelection::caret(2);
            })
            .unwrap();
        assert!(
            !context
                .goto_focused_text_matching_bracket(document)
                .unwrap()
        );
    }

    #[test]
    fn sort_focused_text_lines_sorts_dedups_and_emits_one_change() {
        let mut context = AppContext::new();
        let document = DocumentId::new(1).unwrap();
        let area = context
            .create_component(document, TextArea::new("pear\napple\npear"))
            .unwrap();
        let node = area.stable_id();
        assert!(context.focus_node(document, node).unwrap());
        let events = track_text_changed(&mut context, area);
        context
            .update_component(area, |area, _cx| {
                area.state.selection = TextSelection::new(0, "pear\napple\npear".len());
            })
            .unwrap();

        // 升序 + 去重，选区覆盖排序后的块。
        assert!(
            context
                .sort_focused_text_lines(document, false, true)
                .unwrap()
        );
        assert_eq!(
            textarea_selection(&context, node),
            ("apple\npear".into(), 0, 10)
        );
        assert_eq!(*events.lock().unwrap(), vec!["apple\npear".to_string()]);

        // 降序还原顺序差异。
        context
            .update_component(area, |area, _cx| {
                area.state.selection = TextSelection::new(0, "apple\npear".len());
            })
            .unwrap();
        assert!(
            context
                .sort_focused_text_lines(document, true, false)
                .unwrap()
        );
        assert_eq!(textarea_selection(&context, node).0, "pear\napple");

        // 单行无变化：拒绝且不发事件。
        context
            .update_component(area, |area, _cx| {
                area.state.selection = TextSelection::caret(2);
            })
            .unwrap();
        assert!(
            !context
                .sort_focused_text_lines(document, false, false)
                .unwrap()
        );
        assert_eq!(events.lock().unwrap().len(), 2);
    }

    fn textarea_selections(
        context: &AppContext,
        node: StableNodeId,
    ) -> (String, (usize, usize), Vec<(usize, usize)>) {
        let state = context.world().text_input(node).unwrap();
        (
            state.value.to_owned(),
            (state.selection.anchor, state.selection.focus),
            state
                .additional_selections
                .iter()
                .map(|selection| (selection.anchor, selection.focus))
                .collect(),
        )
    }

    fn set_selections(
        context: &mut AppContext,
        area: Entity<TextArea>,
        primary: (usize, usize),
        additional: Vec<(usize, usize)>,
    ) {
        context
            .update_component(area, |area, _cx| {
                area.state.selection = TextSelection::new(primary.0, primary.1);
                area.state.additional_selections = additional
                    .into_iter()
                    .map(|(anchor, focus)| TextSelection::new(anchor, focus))
                    .collect();
            })
            .unwrap();
    }

    #[test]
    fn editor_render_options_default_off_and_opt_in_drives_derived_presentation() {
        // 四个渲染选项默认关闭：未开启时不产生任何派生标记。
        let defaults = TextArea::new("alpha beta\nalpha");
        assert!(!defaults.occurrence_highlight);
        assert!(!defaults.relative_line_numbers);
        assert!(!defaults.show_whitespace);
        assert!(defaults.wrap_guides.is_empty());

        let mut context = AppContext::new();
        let document = DocumentId::new(1).unwrap();
        let area = context
            .create_component(
                document,
                TextArea::new("alpha beta\nalpha")
                    .line_numbers(true)
                    .relative_line_numbers(true)
                    .occurrence_highlight(true)
                    .show_whitespace(true)
                    .wrap_guides(std::sync::Arc::from([4])),
            )
            .unwrap();
        let node = area.stable_id();
        assert!(context.focus_node(document, node).unwrap());
        let mut layout = MutationQueue::new();
        layout.write_layout(
            node,
            LayoutBox {
                x: 0.0,
                y: 0.0,
                width: 200.0,
                height: 40.0,
            },
        );
        context.commit_mutations(layout).unwrap();
        let work = context.take_system_work();
        context
            .compat_world_mut()
            .resolve_styles(&work.style)
            .unwrap();
        context
            .compat_world_mut()
            .shape_text(&work.text, &mut MeasureTextShaper)
            .unwrap();

        let presentation = context
            .world()
            .text_input_presentation(node)
            .expect("presentation");
        // 出现高亮：光标（值末尾）停在第二行 "alpha" 上，该出现不画；
        // 全词匹配排除前缀，只剩第一行的 "alpha" 一条。
        assert_eq!(presentation.occurrence_marks.len(), 1);
        // 空白显示：行内一个空格（"alpha beta"），换行不标记。
        assert_eq!(presentation.whitespace_marks.len(), 1);
        // wrap guide：列 4 一个 x 位置。
        assert_eq!(presentation.wrap_guides.len(), 1);
        // 相对行号：光标（值末尾）在第 2 行，显示绝对 2；第 1 行距离 1。
        assert_eq!(presentation.line_numbers, vec![1, 2]);
    }

    #[test]
    fn textarea_wheel_scrolls_internal_text_and_survives_projection() {
        let mut context = AppContext::new();
        let document = DocumentId::new(1).unwrap();
        let area = context
            .create_component(document, TextArea::new("line\n".repeat(120)))
            .unwrap();
        let node = area.stable_id();
        let mut layout = MutationQueue::new();
        layout.write_layout(
            node,
            LayoutBox {
                x: 0.0,
                y: 0.0,
                width: 200.0,
                height: 80.0,
            },
        );
        context.commit_mutations(layout).unwrap();
        let work = context.take_system_work();
        context
            .compat_world_mut()
            .resolve_styles(&work.style)
            .unwrap();
        context
            .compat_world_mut()
            .shape_text(&work.text, &mut MeasureTextShaper)
            .unwrap();
        context.rebuild_hit_test(document);
        let event = InputEvent::Wheel {
            x: 60.0,
            y: 40.0,
            delta_x: 0.0,
            delta_y: -120.0,
            line_delta: false,
            modifiers: Default::default(),
        };
        assert!(
            RuntimeInputAdapter::default()
                .dispatch(&mut context, document, &event)
                .unwrap()
                .prevent_default
        );
        assert_eq!(
            context.read(area, |area| area.scroll_offset.y).unwrap(),
            120.0
        );
        context
            .update_component(area, |area, _| area.invalid = true)
            .unwrap();
        assert_eq!(context.world().scroll_offset(node).unwrap().y, 120.0);
    }

    #[test]
    fn pointer_press_on_fold_gutter_toggles_the_fold() {
        let value = "fn a() {\n    x();\n    y();\n}\nfn b() {}";
        let fold = nana_ui_runtime::TextCodeFold::new(7, 28);
        let mut context = AppContext::new();
        let document = DocumentId::new(1).unwrap();
        let mut style = nana_ui_runtime::NodeStyle::default();
        std::sync::Arc::make_mut(&mut style.layout).padding_left =
            Some(nana_ui_core::LengthSpec::Px(46.0));
        let area = context
            .create_component(
                document,
                TextArea::new(value)
                    .code_editor(true)
                    .line_numbers(true)
                    .code_folds(std::sync::Arc::from([fold]))
                    .style(style),
            )
            .unwrap();
        let node = area.stable_id();
        assert!(context.focus_node(document, node).unwrap());
        let mut layout = MutationQueue::new();
        layout.write_layout(
            node,
            LayoutBox {
                x: 0.0,
                y: 0.0,
                width: 200.0,
                height: 40.0,
            },
        );
        context.commit_mutations(layout).unwrap();
        let work = context.take_system_work();
        context
            .compat_world_mut()
            .resolve_styles(&work.style)
            .unwrap();
        context
            .compat_world_mut()
            .shape_text(&work.text, &mut MeasureTextShaper)
            .unwrap();
        context.rebuild_hit_test(document);
        let mut shaper = MeasureTextShaper;
        let mut adapter = RuntimeInputAdapter::default();

        // 光标移到折叠起始行，避免聚焦滚动把该行推出视口。
        context
            .update_component(area, |area_view, _| {
                area_view.state.selection = nana_ui_runtime::TextSelection::caret(3);
            })
            .unwrap();
        let work = context.take_system_work();
        context
            .compat_world_mut()
            .resolve_styles(&work.style)
            .unwrap();
        context
            .compat_world_mut()
            .shape_text(&work.text, &mut MeasureTextShaper)
            .unwrap();
        context.rebuild_hit_test(document);

        let click = |x: f32, y: f32, phase: PointerPhase| InputEvent::Pointer {
            phase,
            pointer_id: 7,
            pointer_type: PointerType::Mouse,
            x,
            y,
            screen_x: x,
            screen_y: y,
            button: 0,
            buttons: u16::from(phase == PointerPhase::Down || phase == PointerPhase::Move),
            pressure: 1.0,
            tangential_pressure: 0.0,
            tilt_x: 0,
            tilt_y: 0,
            twist: 0,
            is_primary: true,
            activation_click: false,
            modifiers: InputModifiers::default(),
        };

        // 折叠前：文档末行的 reveal 行距按 5 行计。
        let reveal_before = context
            .world()
            .text_input_reveal_scroll(node, value.len())
            .unwrap();
        let gutter = context
            .world()
            .component_geometry(node)
            .and_then(|geometry| match geometry {
                nana_ui_runtime::ComponentGeometry::TextInput { folds, .. } => {
                    folds.gutters.first().copied()
                }
                _ => None,
            })
            .expect("fold gutter geometry");
        let center = (
            gutter.bounds.x + gutter.bounds.width / 2.0,
            gutter.bounds.y + gutter.bounds.height / 2.0,
        );

        // 点击 gutter 箭头：折叠该区间（消费事件、不落光标）。
        assert!(
            context
                .pointer_target(document, center.0, center.1)
                .is_some(),
            "no hit target at {:?}",
            center
        );
        assert!(
            adapter
                .dispatch_with_shaper(
                    &mut context,
                    document,
                    &click(center.0, center.1, PointerPhase::Down),
                    Duration::from_millis(1_000),
                    Some(&mut shaper),
                )
                .unwrap()
                .prevent_default
        );
        adapter
            .dispatch_with_shaper(
                &mut context,
                document,
                &click(center.0, center.1, PointerPhase::Up),
                Duration::from_millis(1_010),
                Some(&mut shaper),
            )
            .unwrap();
        assert_eq!(context.world().text_fold_collapsed(node), vec![fold]);
        let work = context.take_system_work();
        context
            .compat_world_mut()
            .resolve_styles(&work.style)
            .unwrap();
        context
            .compat_world_mut()
            .shape_text(&work.text, &mut MeasureTextShaper)
            .unwrap();

        // 折叠后渲染行数减少：同一偏移的 reveal 行距按显示视图换算变小。
        let reveal_after = context
            .world()
            .text_input_reveal_scroll(node, value.len())
            .unwrap();
        assert!(reveal_after.y < reveal_before.y);

        // 再次点击箭头：展开。
        let gutter = context
            .world()
            .component_geometry(node)
            .and_then(|geometry| match geometry {
                nana_ui_runtime::ComponentGeometry::TextInput { folds, .. } => {
                    folds.gutters.first().copied()
                }
                _ => None,
            })
            .expect("fold gutter geometry");
        let center = (
            gutter.bounds.x + gutter.bounds.width / 2.0,
            gutter.bounds.y + gutter.bounds.height / 2.0,
        );
        assert!(
            adapter
                .dispatch_with_shaper(
                    &mut context,
                    document,
                    &click(center.0, center.1, PointerPhase::Down),
                    Duration::from_millis(2_000),
                    Some(&mut shaper),
                )
                .unwrap()
                .prevent_default
        );
        adapter
            .dispatch_with_shaper(
                &mut context,
                document,
                &click(center.0, center.1, PointerPhase::Up),
                Duration::from_millis(2_010),
                Some(&mut shaper),
            )
            .unwrap();
        assert!(context.world().text_fold_collapsed(node).is_empty());
    }

    #[test]
    fn escape_collapses_additional_cursors_and_single_cursor_escape_passes_through() {
        let mut context = AppContext::new();
        let document = DocumentId::new(1).unwrap();
        let area = context
            .create_component(document, TextArea::new("ab\ncd\nef"))
            .unwrap();
        let node = area.stable_id();
        assert!(context.focus_node(document, node).unwrap());
        let mut adapter = RuntimeInputAdapter::default();
        let escape = || InputEvent::Keyboard {
            pressed: true,
            key: "Escape".into(),
            text: None,
            code: "Escape".into(),
            repeat: false,
            modifiers: InputModifiers::default(),
        };

        // 多光标：Esc 塌缩到主光标并消费事件。
        set_selections(&mut context, area, (1, 1), vec![(4, 4)]);
        assert!(
            adapter
                .dispatch(&mut context, document, &escape())
                .unwrap()
                .prevent_default
        );
        assert_eq!(
            textarea_selections(&context, node),
            ("ab\ncd\nef".into(), (1, 1), vec![])
        );

        // 单光标：Esc 不消费（宿主继续处理），选区不变。
        assert!(
            !adapter
                .dispatch(&mut context, document, &escape())
                .unwrap()
                .prevent_default
        );
        assert_eq!(
            textarea_selections(&context, node),
            ("ab\ncd\nef".into(), (1, 1), vec![])
        );
    }

    #[test]
    fn escape_ends_snippet_session_before_collapsing_cursors() {
        let mut context = AppContext::new();
        let document = DocumentId::new(1).unwrap();
        let area = context
            .create_component(document, TextArea::new("ab\ncd"))
            .unwrap();
        let node = area.stable_id();
        assert!(context.focus_node(document, node).unwrap());
        set_selections(&mut context, area, (0, 0), vec![(5, 5)]);
        assert!(
            context
                .insert_focused_text_snippet(
                    document,
                    &nana_ui_runtime::TextSnippet::new("s", "[$1]$0"),
                )
                .unwrap()
        );
        let mut adapter = RuntimeInputAdapter::default();
        let escape = InputEvent::Keyboard {
            pressed: true,
            key: "Escape".into(),
            text: None,
            code: "Escape".into(),
            repeat: false,
            modifiers: InputModifiers::default(),
        };

        // 第一个 Esc：只结束 snippet 会话，多光标保留。
        assert!(
            adapter
                .dispatch(&mut context, document, &escape)
                .unwrap()
                .prevent_default
        );
        assert_eq!(
            textarea_selections(&context, node),
            ("[]ab\ncd".into(), (2, 2), vec![(7, 7)])
        );

        // 第二个 Esc：塌缩多光标。
        assert!(
            adapter
                .dispatch(&mut context, document, &escape)
                .unwrap()
                .prevent_default
        );
        assert_eq!(
            textarea_selections(&context, node),
            ("[]ab\ncd".into(), (2, 2), vec![])
        );
    }

    #[test]
    fn snippet_session_tab_routes_through_the_adapter_before_indent() {
        let mut context = AppContext::new();
        let document = DocumentId::new(1).unwrap();
        let area = context
            .create_component(document, TextArea::new("").code_editor(true))
            .unwrap();
        let node = area.stable_id();
        assert!(context.focus_node(document, node).unwrap());
        assert!(
            context
                .insert_focused_text_snippet(
                    document,
                    &nana_ui_runtime::TextSnippet::new("if", "if $1 {$0"),
                )
                .unwrap()
        );
        let mut adapter = RuntimeInputAdapter::default();

        // 会话内 Tab 跳位（消费且不插入缩进）。
        assert!(
            adapter
                .dispatch(&mut context, document, &plain_key("Tab"))
                .unwrap()
                .prevent_default
        );
        assert_eq!(textarea_selection(&context, node), ("if  {".into(), 3, 3));

        // 会话结束后 Tab 回到缩进行为。
        assert!(
            adapter
                .dispatch(&mut context, document, &plain_key("Tab"))
                .unwrap()
                .prevent_default
        );
        assert!(
            adapter
                .dispatch(&mut context, document, &plain_key("Tab"))
                .unwrap()
                .prevent_default
        );
        assert_ne!(textarea_selection(&context, node).0, "if  {");
    }

    #[test]
    fn multi_cursor_typing_deleting_and_newline_edit_every_selection() {
        let mut context = AppContext::new();
        let document = DocumentId::new(1).unwrap();
        let area = context
            .create_component(document, TextArea::new("ab\ncd"))
            .unwrap();
        let node = area.stable_id();
        assert!(context.focus_node(document, node).unwrap());
        let events = track_text_changed(&mut context, area);
        let mut adapter = RuntimeInputAdapter::default();
        set_selections(&mut context, area, (1, 1), vec![(4, 4)]);

        // 打字：每个光标各插入一个字符，一次事件。
        assert!(
            adapter
                .dispatch(
                    &mut context,
                    document,
                    &edit_key("x", Some("x"), InputModifiers::default())
                )
                .unwrap()
                .prevent_default
        );
        assert_eq!(
            textarea_selections(&context, node),
            ("axb\ncxd".into(), (2, 2), vec![(6, 6)])
        );
        assert_eq!(events.lock().unwrap().len(), 1);

        // 退格：每个光标各删除一个字符。
        assert!(
            adapter
                .dispatch(
                    &mut context,
                    document,
                    &edit_key("Backspace", None, InputModifiers::default())
                )
                .unwrap()
                .prevent_default
        );
        assert_eq!(
            textarea_selections(&context, node),
            // 第二个光标删掉 c 后的 x，落在 c、d 之间（偏移 4）。
            ("ab\ncd".into(), (1, 1), vec![(4, 4)])
        );
        assert_eq!(events.lock().unwrap().len(), 2);

        // Enter：每个光标各换一行（无代码编辑，无自动缩进）。
        assert!(
            adapter
                .dispatch(
                    &mut context,
                    document,
                    &edit_key("Enter", Some("\n"), InputModifiers::default())
                )
                .unwrap()
                .prevent_default
        );
        assert_eq!(
            textarea_selections(&context, node),
            ("a\nb\nc\nd".into(), (2, 2), vec![(6, 6)])
        );
        assert_eq!(events.lock().unwrap().len(), 3);
    }

    #[test]
    fn alt_cmd_arrows_add_cursors_by_column_skip_duplicates_and_stay_at_edges() {
        let mut context = AppContext::new();
        let document = DocumentId::new(1).unwrap();
        let area = context
            .create_component(document, TextArea::new("abcd\nef\nghij"))
            .unwrap();
        let node = area.stable_id();
        assert!(context.focus_node(document, node).unwrap());
        let mut adapter = RuntimeInputAdapter::default();
        let alt_cmd = InputModifiers {
            alt: true,
            meta: true,
            ..InputModifiers::default()
        };
        set_selections(&mut context, area, (2, 2), vec![]);

        // Alt+Cmd+Down 在下一行按列加光标；列超出则贴到行尾。
        assert!(
            adapter
                .dispatch(
                    &mut context,
                    document,
                    &edit_key("ArrowDown", None, alt_cmd)
                )
                .unwrap()
                .prevent_default
        );
        assert_eq!(
            textarea_selections(&context, node),
            ("abcd\nef\nghij".into(), (2, 2), vec![(7, 7)])
        );

        // 再按一次：第二个光标下方按列对齐，第一行光标的候选与已有重复合。
        assert!(
            adapter
                .dispatch(
                    &mut context,
                    document,
                    &edit_key("ArrowDown", None, alt_cmd)
                )
                .unwrap()
                .prevent_default
        );
        assert_eq!(
            textarea_selections(&context, node),
            ("abcd\nef\nghij".into(), (2, 2), vec![(7, 7), (10, 10)])
        );

        // 文档边缘：手势仍被消费，但不再新增光标。
        assert!(
            adapter
                .dispatch(
                    &mut context,
                    document,
                    &edit_key("ArrowDown", None, alt_cmd)
                )
                .unwrap()
                .prevent_default
        );
        assert_eq!(
            textarea_selections(&context, node).2,
            vec![(7, 7), (10, 10)]
        );

        // Alt+Cmd+Up 回程同样按列对齐（10 -> 7 -> 2 依次被已有光标去重）。
        assert!(
            adapter
                .dispatch(&mut context, document, &edit_key("ArrowUp", None, alt_cmd))
                .unwrap()
                .prevent_default
        );
        assert_eq!(
            textarea_selections(&context, node).2,
            vec![(7, 7), (10, 10)]
        );
    }

    #[test]
    fn cmd_d_selects_occurrences_wrapping_and_skipping_covered_spans() {
        let mut context = AppContext::new();
        let document = DocumentId::new(1).unwrap();
        let area = context
            .create_component(document, TextArea::new("ab cd ab"))
            .unwrap();
        let node = area.stable_id();
        assert!(context.focus_node(document, node).unwrap());
        let mut adapter = RuntimeInputAdapter::default();
        let meta = InputModifiers {
            meta: true,
            ..InputModifiers::default()
        };
        set_selections(&mut context, area, (0, 2), vec![]);

        // Cmd+D 选中下一个 "ab"（全词匹配）。
        assert!(
            adapter
                .dispatch(&mut context, document, &edit_key("d", None, meta))
                .unwrap()
                .prevent_default
        );
        assert_eq!(
            textarea_selections(&context, node),
            ("ab cd ab".into(), (0, 2), vec![(6, 8)])
        );

        // 全部出现都已有光标：不再新增（键不被消费）。
        assert!(
            !adapter
                .dispatch(&mut context, document, &edit_key("d", None, meta))
                .unwrap()
                .prevent_default
        );

        // 环形：只留末尾选区时，Cmd+D 绕回文档开头。
        set_selections(&mut context, area, (6, 8), vec![]);
        assert!(
            adapter
                .dispatch(&mut context, document, &edit_key("d", None, meta))
                .unwrap()
                .prevent_default
        );
        assert_eq!(
            textarea_selections(&context, node),
            ("ab cd ab".into(), (6, 8), vec![(0, 2)])
        );

        // 全部选中：裸光标取光标下的词，选中所有出现。
        set_selections(&mut context, area, (1, 1), vec![]);
        assert!(
            context
                .select_all_focused_text_occurrences(document)
                .unwrap()
        );
        // 裸光标被它所在的词选区吸收（并集后主光标即该词）。
        assert_eq!(
            textarea_selections(&context, node),
            ("ab cd ab".into(), (0, 2), vec![(6, 8)])
        );

        // 收回到主光标；再次收回是空操作。
        assert!(context.collapse_focused_text_selections(document).unwrap());
        assert_eq!(textarea_selections(&context, node).2, vec![]);
        assert!(!context.collapse_focused_text_selections(document).unwrap());
    }

    #[test]
    fn copy_joins_multi_cursor_selections_and_paste_hits_every_cursor() {
        let mut context = AppContext::new();
        let document = DocumentId::new(1).unwrap();
        let area = context
            .create_component(document, TextArea::new("ab cd"))
            .unwrap();
        let node = area.stable_id();
        assert!(context.focus_node(document, node).unwrap());
        let clipboard = shared_clipboard(MemoryClipboard::new());
        let mut adapter = RuntimeInputAdapter::default().with_clipboard(Arc::clone(&clipboard));
        let meta = |key: &str| {
            edit_key(
                key,
                None,
                InputModifiers {
                    meta: true,
                    ..InputModifiers::default()
                },
            )
        };
        set_selections(&mut context, area, (0, 2), vec![(3, 5)]);

        // Cmd+C：多选区按序拼接（Zed 语义，换行连接）。
        assert!(
            adapter
                .dispatch(&mut context, document, &meta("c"))
                .unwrap()
                .prevent_default
        );
        assert_eq!(
            clipboard.lock().unwrap().read_text().as_deref(),
            Some("ab\ncd")
        );

        // Cmd+V：同一段文本插入到每个光标。
        set_selections(&mut context, area, (0, 0), vec![(5, 5)]);
        assert!(
            adapter
                .dispatch(&mut context, document, &meta("v"))
                .unwrap()
                .prevent_default
        );
        assert_eq!(
            textarea_selections(&context, node),
            ("ab\ncdab cdab\ncd".into(), (5, 5), vec![(15, 15)])
        );

        // Cmd+X：多选区剪切一并删除。
        set_selections(&mut context, area, (0, 2), vec![(8, 10)]);
        assert!(
            adapter
                .dispatch(&mut context, document, &meta("x"))
                .unwrap()
                .prevent_default
        );
        assert_eq!(textarea_selections(&context, node).0, "\ncdab ab\ncd");
    }

    #[test]
    fn ime_commit_scopes_to_the_primary_cursor_and_remaps_others() {
        let mut context = AppContext::new();
        let document = DocumentId::new(1).unwrap();
        let area = context
            .create_component(document, TextArea::new("ab\ncd"))
            .unwrap();
        let node = area.stable_id();
        assert!(context.focus_node(document, node).unwrap());
        let adapter = RuntimeInputAdapter::default();
        set_selections(&mut context, area, (2, 2), vec![(4, 4)]);

        assert!(
            adapter
                .dispatch_ime(&mut context, document, &ImeEvent::Commit("X".into()))
                .unwrap()
                .prevent_default
        );
        // 只有主光标收到提交文本，附加光标随编辑平移。
        assert_eq!(
            textarea_selections(&context, node),
            ("abX\ncd".into(), (3, 3), vec![(5, 5)])
        );
    }

    #[test]
    fn single_line_fields_reject_multi_cursor_gestures() {
        let mut context = AppContext::new();
        let document = DocumentId::new(1).unwrap();
        let input = context
            .create_component(document, TextInput::new("hi"))
            .unwrap();
        let node = input.stable_id();
        assert!(context.focus_node(document, node).unwrap());
        let mut adapter = RuntimeInputAdapter::default();
        let alt_cmd = InputModifiers {
            alt: true,
            meta: true,
            ..InputModifiers::default()
        };
        let meta = InputModifiers {
            meta: true,
            ..InputModifiers::default()
        };

        // 命令层直接拒绝。
        assert!(
            !context
                .add_focused_text_cursor(document, false, None)
                .unwrap()
        );
        assert!(
            !context
                .select_focused_text_occurrence(document, false)
                .unwrap()
        );

        // Alt+Cmd+Down 回落到普通移动（meta=DocEnd），不加光标。
        // 先把光标挪到行首，DocEnd 才有位移。
        context
            .update_component(input, |input, _cx| {
                input.state.selection = TextSelection::caret(0);
            })
            .unwrap();
        assert!(
            adapter
                .dispatch(
                    &mut context,
                    document,
                    &edit_key("ArrowDown", None, alt_cmd)
                )
                .unwrap()
                .prevent_default
        );
        assert_eq!(textarea_selections(&context, node).1, (2, 2));
        assert_eq!(textarea_selections(&context, node).2, vec![]);

        // Cmd+D 不消费、不产生附加光标。
        assert!(
            !adapter
                .dispatch(&mut context, document, &edit_key("d", None, meta))
                .unwrap()
                .prevent_default
        );
        assert_eq!(textarea_selections(&context, node).2, vec![]);
    }

    #[test]
    fn alt_click_adds_and_removes_cursors_and_plain_click_collapses() {
        let mut context = AppContext::new();
        let document = DocumentId::new(1).unwrap();
        let area = context
            .create_component(document, TextArea::new("first\nsecond"))
            .unwrap();
        let node = area.stable_id();
        let mut layout = MutationQueue::new();
        layout.write_layout(
            node,
            LayoutBox {
                x: 0.0,
                y: 0.0,
                width: 200.0,
                height: 64.0,
            },
        );
        layout.set_standard_visual(
            node,
            Some(nana_ui_runtime::StandardVisual::TextInput {
                placeholder: std::sync::Arc::from(""),
                size: nana_ui_core::ControlSize::Medium,
                secure: false,
                invalid: false,
                steppers: false,
                diagnostics: std::sync::Arc::from([]),
                matches: std::sync::Arc::from([]),
                color_swatches: std::sync::Arc::from([]),
                atoms: Arc::from([]),
                line_numbers: false,
                indent_guides: None,
                folds: std::sync::Arc::from([]),
                git_marks: std::sync::Arc::from([]),
                editor_options: Default::default(),
            }),
        );
        context.commit_mutations(layout).unwrap();
        context.take_system_work();
        context.rebuild_hit_test(document);

        let mut shaper = MeasureTextShaper;
        let mut adapter = RuntimeInputAdapter::default();
        let click = |x: f32, y: f32, alt: bool| InputEvent::Pointer {
            phase: PointerPhase::Down,
            pointer_id: 7,
            pointer_type: PointerType::Mouse,
            x,
            y,
            screen_x: x,
            screen_y: y,
            button: 0,
            buttons: 1,
            pressure: 1.0,
            tangential_pressure: 0.0,
            tilt_x: 0,
            tilt_y: 0,
            twist: 0,
            is_primary: true,
            activation_click: false,
            modifiers: if alt {
                InputModifiers {
                    alt: true,
                    ..InputModifiers::default()
                }
            } else {
                InputModifiers::default()
            },
        };
        struct Ctx<'a> {
            context: &'a mut AppContext,
            shaper: &'a mut MeasureTextShaper,
        }
        let mut ctx = Ctx {
            context: &mut context,
            shaper: &mut shaper,
        };
        let mut dispatch_down = |ctx: &mut Ctx, event: &InputEvent, at: u64| -> bool {
            adapter
                .dispatch_with_shaper(
                    ctx.context,
                    document,
                    event,
                    Duration::from_millis(at),
                    Some(&mut *ctx.shaper),
                )
                .unwrap()
                .prevent_default
        };

        // 先用普通点击探出 (2, 20) 落点的字符偏移（不依赖具体行高）。
        assert!(dispatch_down(&mut ctx, &click(2.0, 20.0, false), 1_000));
        let probe = textarea_selections(ctx.context, node).1;
        assert_eq!(probe.0, probe.1);
        // 把主光标挪到文档末尾，让目标点空出来。
        ctx.context
            .update_component(area, |area, _cx| {
                area.state.selection = TextSelection::caret("first\nsecond".len());
            })
            .unwrap();

        // Alt+点击同一点：新增一个光标。
        assert!(dispatch_down(&mut ctx, &click(2.0, 20.0, true), 2_000));
        assert_eq!(
            textarea_selections(ctx.context, node),
            ("first\nsecond".into(), (12, 12), vec![probe])
        );

        // 时间错开避免双击判定；再次 Alt+点击同一点：移除该光标。
        assert!(dispatch_down(&mut ctx, &click(2.0, 20.0, true), 3_000));
        assert_eq!(textarea_selections(ctx.context, node).2, vec![]);

        // Alt+点击第一行行首（主光标不在该处）：新增光标。
        assert!(dispatch_down(&mut ctx, &click(2.0, 4.0, true), 4_000));
        assert_eq!(textarea_selections(ctx.context, node).2, vec![(0, 0)]);

        // 普通点击：天然塌缩回单光标。
        assert!(dispatch_down(&mut ctx, &click(2.0, 4.0, false), 5_000));
        assert_eq!(
            textarea_selections(ctx.context, node),
            ("first\nsecond".into(), (0, 0), vec![])
        );
    }

    #[test]
    fn multi_cursor_code_editing_indents_comments_moves_lines_and_deletes_words() {
        let mut context = AppContext::new();
        let document = DocumentId::new(1).unwrap();
        let area = context
            .create_component(document, TextArea::new("  a\n  b").code_editor(true))
            .unwrap();
        let node = area.stable_id();
        assert!(context.focus_node(document, node).unwrap());
        let mut adapter = RuntimeInputAdapter::default();
        let control = InputModifiers {
            control: true,
            ..InputModifiers::default()
        };
        let alt = InputModifiers {
            alt: true,
            ..InputModifiers::default()
        };

        // Enter 自动缩进：两个缩进行上的光标各起新行并继承缩进。
        set_selections(&mut context, area, (3, 3), vec![(7, 7)]);
        assert!(
            adapter
                .dispatch(
                    &mut context,
                    document,
                    &edit_key("Enter", Some("\n"), InputModifiers::default())
                )
                .unwrap()
                .prevent_default
        );
        assert_eq!(
            textarea_selections(&context, node),
            ("  a\n  \n  b\n  ".into(), (6, 6), vec![(13, 13)])
        );

        // Ctrl+/ 注释切换：每个光标注释自己所在的行。
        context
            .update_component(area, |area, _cx| {
                area.state = nana_ui_runtime::TextInputState::new("aa\nbb");
                area.state.selection = TextSelection::caret(1);
                area.state.additional_selections = vec![TextSelection::caret(4)];
            })
            .unwrap();
        assert!(
            adapter
                .dispatch(&mut context, document, &edit_key("/", None, control))
                .unwrap()
                .prevent_default
        );
        assert_eq!(
            textarea_selections(&context, node),
            ("//aa\n//bb".into(), (3, 3), vec![(8, 8)])
        );

        // Alt+Backspace 词删除：每个光标删到词首。
        context
            .update_component(area, |area, _cx| {
                area.state = nana_ui_runtime::TextInputState::new("aa\nbb");
                area.state.selection = TextSelection::caret(1);
                area.state.additional_selections = vec![TextSelection::caret(4)];
            })
            .unwrap();
        assert!(
            adapter
                .dispatch(&mut context, document, &edit_key("Backspace", None, alt))
                .unwrap()
                .prevent_default
        );
        assert_eq!(
            textarea_selections(&context, node),
            ("a\nb".into(), (0, 0), vec![(2, 2)])
        );

        // Alt+Down 行移动：首行光标把行下移；末行光标在边缘保持不动。
        context
            .update_component(area, |area, _cx| {
                area.state = nana_ui_runtime::TextInputState::new("aa\nbb\ncc");
                area.state.selection = TextSelection::caret(1);
                area.state.additional_selections = vec![TextSelection::caret(7)];
            })
            .unwrap();
        assert!(
            adapter
                .dispatch(&mut context, document, &edit_key("ArrowDown", None, alt))
                .unwrap()
                .prevent_default
        );
        assert_eq!(
            textarea_selections(&context, node),
            ("bb\naa\ncc".into(), (4, 4), vec![(7, 7)])
        );
    }

    fn completion_items(labels: &[&str]) -> std::sync::Arc<[nana_ui_runtime::TextCompletion]> {
        labels
            .iter()
            .map(|label| nana_ui_runtime::TextCompletion::new(*label, "fn"))
            .collect::<Vec<_>>()
            .into()
    }

    /// 布局 + shape + 命中测试的完整几何环境（指针/滚轮路由需要）。
    fn shape_completion_editor(context: &mut AppContext, document: DocumentId, node: StableNodeId) {
        context.compat_world_mut().resolve_styles(&[node]).unwrap();
        context
            .compat_world_mut()
            .shape_text(&[node], &mut MeasureTextShaper)
            .unwrap();
        context.rebuild_hit_test(document);
    }

    fn completion_popup_geometry(
        context: &AppContext,
        node: StableNodeId,
    ) -> nana_ui_runtime::TextCompletionPopup {
        match context.world().component_geometry(node) {
            Some(nana_ui_runtime::ComponentGeometry::TextInput {
                completion_popup, ..
            }) => completion_popup.expect("completion popup geometry"),
            _ => panic!("text input geometry"),
        }
    }

    #[test]
    fn completion_popup_owns_navigation_and_accept_keys() {
        let mut context = AppContext::new();
        let document = DocumentId::new(1).unwrap();
        let area = context
            .create_component(
                document,
                TextArea::new("let fo").completions(completion_items(&["food", "foobar"])),
            )
            .unwrap();
        let node = area.stable_id();
        assert!(context.focus_node(document, node).unwrap());
        set_selections(&mut context, area, (6, 6), vec![]);
        let mut adapter = RuntimeInputAdapter::default();
        let selected = |context: &AppContext, node| {
            context
                .world()
                .text_completion_snapshot(node)
                .map(|snapshot| (snapshot.selected, snapshot.dismissed))
        };

        // Down：弹层消费（选区不动），候选选中移到第二条。
        assert!(
            adapter
                .dispatch(&mut context, document, &plain_key("ArrowDown"))
                .unwrap()
                .prevent_default
        );
        assert_eq!(textarea_selection(&context, node), ("let fo".into(), 6, 6));
        assert_eq!(selected(&context, node), Some((1, false)));

        // Up：回到第一条。
        assert!(
            adapter
                .dispatch(&mut context, document, &plain_key("ArrowUp"))
                .unwrap()
                .prevent_default
        );
        assert_eq!(selected(&context, node), Some((0, false)));

        // Enter：接受选中项，一次 TextChanged，光标落在插入末尾。
        let changes = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let sink = std::sync::Arc::clone(&changes);
        context
            .on(area, move |_view, event: &TextChanged, _cx| {
                sink.lock().unwrap().push(event.value.to_string());
            })
            .unwrap();
        assert!(
            adapter
                .dispatch(&mut context, document, &plain_key("Enter"))
                .unwrap()
                .prevent_default
        );
        assert_eq!(
            textarea_selection(&context, node),
            ("let food".into(), 8, 8)
        );
        assert_eq!(*changes.lock().unwrap(), vec!["let food".to_string()]);

        // 宿主重喂（组件重投影）：会话重新激活，Tab 同样接受。
        context
            .update_component(area, |view, _| {
                view.completions = completion_items(&["food"]);
            })
            .unwrap();
        assert_eq!(selected(&context, node), Some((0, false)));
        assert!(
            adapter
                .dispatch(&mut context, document, &plain_key("Tab"))
                .unwrap()
                .prevent_default
        );
        assert_eq!(
            textarea_selection(&context, node),
            ("let food".into(), 8, 8)
        );

        // 打字穿透正常编辑：committed value 直接更新（弹层保持，过滤
        // 由宿主重喂驱动）。
        context
            .update_component(area, |view, _| {
                view.completions = completion_items(&["food"]);
            })
            .unwrap();
        assert!(
            adapter
                .dispatch(
                    &mut context,
                    document,
                    &edit_key("s", Some("s"), InputModifiers::default())
                )
                .unwrap()
                .prevent_default
        );
        assert_eq!(
            textarea_selection(&context, node),
            ("let foods".into(), 9, 9)
        );
    }

    #[test]
    fn modified_keys_pass_through_while_completion_active() {
        let mut context = AppContext::new();
        let document = DocumentId::new(1).unwrap();
        let area = context
            .create_component(
                document,
                TextArea::new("ab ab").completions(completion_items(&["ab"])),
            )
            .unwrap();
        let node = area.stable_id();
        assert!(context.focus_node(document, node).unwrap());
        set_selections(&mut context, area, (1, 1), vec![]);
        let mut adapter = RuntimeInputAdapter::default();

        // Cmd+D 穿透：选中下一出现（多光标 +1），弹层保持。
        let meta_d = edit_key(
            "d",
            None,
            InputModifiers {
                meta: true,
                ..InputModifiers::default()
            },
        );
        assert!(
            adapter
                .dispatch(&mut context, document, &meta_d)
                .unwrap()
                .prevent_default
        );
        assert_eq!(
            textarea_selections(&context, node),
            ("ab ab".into(), (1, 1), vec![(3, 5)])
        );
        assert!(
            context
                .world()
                .text_completion_snapshot(node)
                .is_some_and(|snapshot| !snapshot.dismissed)
        );
    }

    #[test]
    fn escape_closes_completion_after_snippet_and_before_collapse() {
        let mut context = AppContext::new();
        let document = DocumentId::new(1).unwrap();
        let area = context
            .create_component(
                document,
                TextArea::new("ab\ncd").completions(completion_items(&["ab"])),
            )
            .unwrap();
        let node = area.stable_id();
        assert!(context.focus_node(document, node).unwrap());
        set_selections(&mut context, area, (0, 0), vec![(5, 5)]);
        assert!(
            context
                .insert_focused_text_snippet(
                    document,
                    &nana_ui_runtime::TextSnippet::new("s", "[$1]$0"),
                )
                .unwrap()
        );
        // snippet 插入后宿主重喂（组件重投影路径）：弹层重新激活。
        context
            .update_component(area, |view, _| {
                view.completions = completion_items(&["ab"]);
            })
            .unwrap();
        let mut adapter = RuntimeInputAdapter::default();
        let escape = InputEvent::Keyboard {
            pressed: true,
            key: "Escape".into(),
            text: None,
            code: "Escape".into(),
            repeat: false,
            modifiers: InputModifiers::default(),
        };

        // 第一个 Esc：结束 snippet 会话（弹层与多光标保留）。
        assert!(
            adapter
                .dispatch(&mut context, document, &escape)
                .unwrap()
                .prevent_default
        );
        assert!(
            context
                .world()
                .text_completion_snapshot(node)
                .is_some_and(|snapshot| !snapshot.dismissed)
        );
        assert_eq!(textarea_selections(&context, node).2.len(), 1);

        // 第二个 Esc：关闭补全弹层（多光标保留）。
        assert!(
            adapter
                .dispatch(&mut context, document, &escape)
                .unwrap()
                .prevent_default
        );
        assert!(
            context
                .world()
                .text_completion_snapshot(node)
                .is_some_and(|snapshot| snapshot.dismissed)
        );
        assert_eq!(textarea_selections(&context, node).2.len(), 1);

        // 第三个 Esc：塌缩多光标。
        assert!(
            adapter
                .dispatch(&mut context, document, &escape)
                .unwrap()
                .prevent_default
        );
        assert_eq!(textarea_selections(&context, node).2, vec![]);
    }

    #[test]
    fn completion_click_accepts_row_and_wheel_scrolls_overlay() {
        let mut context = AppContext::new();
        let document = DocumentId::new(1).unwrap();
        let area = context
            .create_component(
                document,
                TextArea::new("")
                    .completions(completion_items(&["alpha", "beta", "gamma", "delta"])),
            )
            .unwrap();
        let node = area.stable_id();
        assert!(context.focus_node(document, node).unwrap());
        let mut layout = MutationQueue::new();
        layout.write_layout(
            node,
            LayoutBox {
                x: 0.0,
                y: 0.0,
                width: 200.0,
                height: 140.0,
            },
        );
        context.commit_mutations(layout).unwrap();
        shape_completion_editor(&mut context, document, node);

        // 点击弹层第二行：接受该候选（beta），不落光标。
        let popup = completion_popup_geometry(&context, node);
        let row = &popup.rows[1];
        let click = |x: f32, y: f32, phase: PointerPhase| InputEvent::Pointer {
            phase,
            pointer_id: 7,
            pointer_type: PointerType::Mouse,
            x,
            y,
            screen_x: x,
            screen_y: y,
            button: 0,
            buttons: u16::from(phase == PointerPhase::Down),
            pressure: 1.0,
            tangential_pressure: 0.0,
            tilt_x: 0,
            tilt_y: 0,
            twist: 0,
            is_primary: true,
            activation_click: false,
            modifiers: InputModifiers::default(),
        };
        let mut shaper = MeasureTextShaper;
        let mut adapter = RuntimeInputAdapter::default();
        assert!(
            adapter
                .dispatch_with_shaper(
                    &mut context,
                    document,
                    &click(row.bounds.x + 2.0, row.bounds.y + 2.0, PointerPhase::Down),
                    Duration::ZERO,
                    Some(&mut shaper),
                )
                .unwrap()
                .prevent_default
        );
        assert_eq!(textarea_selection(&context, node), ("beta".into(), 4, 4));

        // 重喂十条候选并重建几何：滚轮落在弹层面板内滚动弹层（消费），
        // 不落到编辑器滚动。
        context
            .update_component(area, |view, _| {
                view.completions = completion_items(&[
                    "a1", "a2", "a3", "a4", "a5", "a6", "a7", "a8", "a9", "a10",
                ]);
                view.state.selection = nana_ui_runtime::TextSelection::caret(4);
            })
            .unwrap();
        shape_completion_editor(&mut context, document, node);
        let popup = completion_popup_geometry(&context, node);
        let scroll = |context: &AppContext| {
            context
                .world()
                .text_completion_snapshot(node)
                .map(|snapshot| snapshot.scroll)
        };
        assert!(
            adapter
                .dispatch(
                    &mut context,
                    document,
                    &wheel(popup.panel.x + 3.0, popup.panel.y + 3.0, 3.0)
                )
                .unwrap()
                .prevent_default
        );
        assert_eq!(scroll(&context), Some(1));
        assert!(
            adapter
                .dispatch(
                    &mut context,
                    document,
                    &wheel(popup.panel.x + 3.0, popup.panel.y + 3.0, -3.0)
                )
                .unwrap()
                .prevent_default
        );
        assert_eq!(scroll(&context), Some(0));

        // hover 浮窗滚轮：正文按行滚动并被消费。
        context
            .update_component(area, |view, _| {
                view.hover = Some(nana_ui_runtime::TextHover::new(
                    0,
                    "beta",
                    "one\ntwo\nthree",
                ));
            })
            .unwrap();
        shape_completion_editor(&mut context, document, node);
        let hover_panel = match context.world().component_geometry(node) {
            Some(nana_ui_runtime::ComponentGeometry::TextInput { hover_popup, .. }) => {
                hover_popup.expect("hover popup").panel
            }
            _ => panic!("text input geometry"),
        };
        assert!(
            adapter
                .dispatch(
                    &mut context,
                    document,
                    &wheel(hover_panel.x + 3.0, hover_panel.y + 3.0, 3.0)
                )
                .unwrap()
                .prevent_default
        );
        assert_eq!(context.world().text_hover_scroll(node), 1);
    }

    #[test]
    fn hover_wheel_scrolls_without_editor_focus() {
        let mut context = AppContext::new();
        let document = DocumentId::new(1).unwrap();
        let area = context
            .create_component(
                document,
                TextArea::new("alpha beta").hover(Some(nana_ui_runtime::TextHover::new(
                    6,
                    "beta",
                    "one\ntwo\nthree",
                ))),
            )
            .unwrap();
        let node = area.stable_id();
        let mut layout = MutationQueue::new();
        layout.write_layout(
            node,
            LayoutBox {
                x: 0.0,
                y: 0.0,
                width: 200.0,
                height: 140.0,
            },
        );
        context.commit_mutations(layout).unwrap();
        shape_completion_editor(&mut context, document, node);
        let hover_panel = match context.world().component_geometry(node) {
            Some(nana_ui_runtime::ComponentGeometry::TextInput { hover_popup, .. }) => {
                hover_popup.expect("hover popup").panel
            }
            _ => panic!("text input geometry"),
        };
        let mut adapter = RuntimeInputAdapter::default();

        // 编辑器未聚焦：滚轮落在 hover 面板内仍滚动该面板（命中测试驱动，
        // hover 显示不要求焦点）。
        assert!(
            adapter
                .dispatch(
                    &mut context,
                    document,
                    &wheel(hover_panel.x + 3.0, hover_panel.y + 3.0, 3.0)
                )
                .unwrap()
                .prevent_default
        );
        assert_eq!(context.world().text_hover_scroll(node), 1);

        // 面板外：不消费，落回编辑器/文档滚动。
        assert!(
            !adapter
                .dispatch(&mut context, document, &wheel(-50.0, -50.0, 3.0))
                .unwrap()
                .prevent_default
        );
        assert_eq!(context.world().text_hover_scroll(node), 1);
    }
}

#[cfg(test)]
mod terminal_input_tests {
    use super::*;
    use nana_ui_runtime::{TerminalEvent, TerminalScreen, TerminalView};
    use std::sync::{Arc, Mutex};

    #[test]
    fn terminal_ime_preedit_does_not_send_until_commit_and_focus_is_scoped() {
        let mut context = AppContext::new();
        let document = DocumentId::new(1).unwrap();
        let terminal = context
            .create_component(document, TerminalView::new(TerminalScreen::blank(4, 2)))
            .unwrap();
        let events = Arc::new(Mutex::new(Vec::new()));
        let observed = events.clone();
        context
            .on(terminal, move |_, event: &TerminalEvent, _| {
                observed.lock().unwrap().push(event.clone())
            })
            .unwrap();
        context.focus_node(document, terminal.stable_id()).unwrap();
        let adapter = RuntimeInputAdapter::default();
        assert!(
            adapter
                .dispatch_ime(
                    &mut context,
                    document,
                    &ImeEvent::Preedit {
                        text: "zhong".into(),
                        selection: None
                    }
                )
                .unwrap()
                .prevent_default
        );
        assert!(events.lock().unwrap().is_empty());
        assert!(
            adapter
                .dispatch_ime(&mut context, document, &ImeEvent::Commit("中文".into()))
                .unwrap()
                .prevent_default
        );
        assert_eq!(
            events.lock().unwrap().as_slice(),
            &[TerminalEvent::Input("中文".as_bytes().to_vec())]
        );
        let editor = context
            .create_component(document, nana_ui_runtime::TextInput::new(""))
            .unwrap();
        context.focus_node(document, editor.stable_id()).unwrap();
        adapter
            .dispatch_ime(&mut context, document, &ImeEvent::Commit("字".into()))
            .unwrap();
        assert_eq!(events.lock().unwrap().len(), 1);
    }
}

#[cfg(test)]
mod canonical_router_tests {
    use super::*;
    use nana_ui_platform::{
        CanonicalInputEvent, EndpointGeneration, HostRequestContext, HostServiceOutcome,
        HostServiceRequest, HostServiceResponse, InputMetadata, InputPayload, InputSequence,
        InputSourceId, InputTimestamp,
    };
    use nana_ui_runtime::{InteractionState, LayoutBox, MutationQueue, NodeKind, StableNodeId};
    use std::sync::Arc;

    fn event(
        source: InputSourceId,
        generation: EndpointGeneration,
        payload: InputPayload,
    ) -> CanonicalInputEvent {
        CanonicalInputEvent {
            metadata: InputMetadata {
                source,
                device: nana_ui_platform::DeviceId(1),
                generation,
                sequence: InputSequence(1),
                timestamp: InputTimestamp(1),
            },
            payload,
        }
    }

    fn pointer_event(
        source: InputSourceId,
        generation: EndpointGeneration,
        device: nana_ui_platform::DeviceId,
        sequence: u64,
        phase: PointerPhase,
    ) -> CanonicalInputEvent {
        CanonicalInputEvent {
            metadata: InputMetadata {
                source,
                device,
                generation,
                sequence: InputSequence(sequence),
                timestamp: InputTimestamp(sequence),
            },
            payload: InputPayload::Pointer(nana_ui_platform::PointerInput {
                phase,
                pointer_id: nana_ui_platform::PointerId(42),
                pointer_type: nana_ui_platform::PointerType::Mouse,
                x: 10.0,
                y: 12.0,
                screen_x: 10.0,
                screen_y: 12.0,
                button: 0,
                buttons: if phase == PointerPhase::Down { 1 } else { 0 },
                pressure: 0.5,
                tangential_pressure: 0.0,
                tilt_x: 0,
                tilt_y: 0,
                twist: 0,
                is_primary: true,
                activation_click: false,
                modifiers: Default::default(),
            }),
        }
    }

    #[test]
    fn pointer_metadata_after_enter_survives_metadata_free_events() {
        let mut router = InputRouter::default();
        let source = InputSourceId(31);
        let generation = EndpointGeneration(1);
        let device = nana_ui_platform::DeviceId(1);
        let pointer = nana_ui_platform::PointerId(42);
        let enter = event(
            source,
            generation,
            InputPayload::PointerEnter {
                pointer_id: pointer,
                x: 10.0,
                y: 12.0,
            },
        );
        router.remap_pointer(&enter);
        let local = router.pointers[&(source, device, pointer)].local;
        let mut sample = pointer_event(source, generation, device, 2, PointerPhase::Down);
        let InputPayload::Pointer(data) = &mut sample.payload else {
            unreachable!()
        };
        data.pointer_type = nana_ui_platform::PointerType::Pen;
        data.is_primary = false;
        router.remap_pointer(&sample);
        router.remap_pointer(&enter);
        router.remap_pointer(&event(
            source,
            generation,
            InputPayload::PointerLeave {
                pointer_id: pointer,
            },
        ));
        let identity = router.pointers[&(source, device, pointer)];
        assert_eq!(identity.local, local);
        assert_eq!(identity.pointer_type, nana_ui_platform::PointerType::Pen);
        assert!(!identity.is_primary);
    }

    #[test]
    fn source_lifecycle_bypasses_device_tombstone_without_reviving_device() {
        let mut router = InputRouter::default();
        let source = InputSourceId(32);
        let generation = EndpointGeneration(1);
        router.attach(source, generation, DocumentId::new(1).unwrap());
        let mut context = AppContext::new();
        for (sequence, payload) in [
            (1, InputPayload::DeviceDisconnected),
            (2, InputPayload::SourceDisconnected),
            (3, InputPayload::SourceConnected),
        ] {
            let mut sample = event(source, generation, payload);
            sample.metadata.sequence = nana_ui_platform::InputSequence(sequence);
            router
                .route(&mut context, &sample, Duration::ZERO, None)
                .unwrap();
        }
        let mut sample = event(source, generation, InputPayload::Focus { focused: true });
        sample.metadata.sequence = nana_ui_platform::InputSequence(4);
        assert!(matches!(
            router.route(&mut context, &sample, Duration::ZERO, None),
            Err(InputRouterError::Disconnected)
        ));
        sample.payload = InputPayload::DeviceConnected;
        router
            .route(&mut context, &sample, Duration::ZERO, None)
            .unwrap();
    }

    #[test]
    fn endpoint_generation_is_checked_before_runtime_dispatch() {
        let mut router = InputRouter::default();
        let source = InputSourceId(9);
        let generation = EndpointGeneration(3);
        let document = DocumentId::new(1).unwrap();
        router.attach(source, generation, document);
        let mut context = AppContext::new();
        assert!(matches!(
            router.route(
                &mut context,
                &event(
                    source,
                    EndpointGeneration(2),
                    InputPayload::Focus { focused: true },
                ),
                Duration::ZERO,
                None,
            ),
            Err(InputRouterError::StaleGeneration)
        ));
        assert!(
            router
                .route(
                    &mut context,
                    &event(source, generation, InputPayload::Focus { focused: true }),
                    Duration::ZERO,
                    None
                )
                .is_ok()
        );
        assert_eq!(router.detach(source), Some(document));
        assert!(matches!(
            router.route(
                &mut context,
                &event(source, generation, InputPayload::Focus { focused: true }),
                Duration::ZERO,
                None
            ),
            Err(InputRouterError::UnknownSource)
        ));
        let counters = router.counters();
        assert_eq!(counters.events_routed, 1);
        assert_eq!(counters.events_rejected_stale_generation, 1);
        assert_eq!(counters.events_rejected_unknown_source, 1);
    }

    #[test]
    fn route_outcome_reports_non_consuming_work_and_queue_delta() {
        let mut router = InputRouter::default();
        let source = InputSourceId(8);
        let generation = EndpointGeneration(1);
        let document = DocumentId::new(3).unwrap();
        router.attach(source, generation, document);
        let mut context = AppContext::new();
        context.take_system_work();

        let outcome = router
            .route_with_outcome(
                &mut context,
                &event(source, generation, InputPayload::Focus { focused: false }),
                Duration::ZERO,
                None,
            )
            .unwrap();
        assert_eq!(outcome.prevent_default, outcome.handled);
        assert_eq!(outcome.host_requests_enqueued, 0);
        assert!(!outcome.invalidated_work);
        // Reading the result did not consume the Runtime queue.
        assert_eq!(context.world().pending_work_revision(), 0);
    }

    #[test]
    fn empty_endpoint_is_idle_without_router_work() {
        let mut router = InputRouter::default();
        let source = InputSourceId(22);
        let generation = EndpointGeneration(1);
        let document = DocumentId::new(15).unwrap();
        router.attach(source, generation, document);
        let mut context = AppContext::new();
        let mut endpoint = nana_ui_platform::InputEndpoint::new(source, generation, 8, 1024);
        assert_eq!(
            router.route_endpoint(&mut context, &mut endpoint, Duration::ZERO),
            Ok(0)
        );
        let counters = router.counters();
        assert_eq!(counters.events_routed, 0);
        assert_eq!(counters.hit_tests, 0);
        assert_eq!(counters.routing_cache_hits, 0);
        assert_eq!(context.world().pending_work_revision(), 0);
    }

    #[test]
    fn router_counters_expose_hit_and_route_work_without_extra_tree_state() {
        let mut router = InputRouter::default();
        let source = InputSourceId(20);
        let generation = EndpointGeneration(1);
        let document = DocumentId::new(14).unwrap();
        router.attach(source, generation, document);
        let mut context = AppContext::new();
        router
            .route(
                &mut context,
                &pointer_event(
                    source,
                    generation,
                    nana_ui_platform::DeviceId(1),
                    1,
                    PointerPhase::Move,
                ),
                Duration::ZERO,
                None,
            )
            .unwrap();
        let counters = router.counters();
        assert_eq!(counters.hit_tests, 1);
        assert_eq!(counters.routing_cache_misses, 1);
        assert_eq!(counters.routing_cache_hits, 0);
        assert_eq!(counters.routed_dispatches, 1);
    }

    #[test]
    fn pointer_route_emits_cursor_intent_only_when_hover_cursor_changes() {
        let mut router = InputRouter::default();
        let source = InputSourceId(21);
        let generation = EndpointGeneration(1);
        let device = nana_ui_platform::DeviceId(1);
        let document = DocumentId::new(17).unwrap();
        router.attach(source, generation, document);
        let mut context = AppContext::new();
        let node = StableNodeId::new(1).unwrap();
        let mut create = MutationQueue::new();
        create.create(
            node,
            document,
            NodeKind::Element {
                tag: "button".into(),
            },
        );
        create.set_interaction(
            node,
            InteractionState {
                pointer_events: true,
                ..InteractionState::default()
            },
        );
        create.set_style(
            node,
            nana_ui_runtime::NodeStyle {
                layout: Arc::new(nana_ui_runtime::LayoutStyle {
                    cursor: Some(nana_ui_core::CursorSpec::Pointer),
                    ..nana_ui_runtime::LayoutStyle::default()
                }),
                ..nana_ui_runtime::NodeStyle::default()
            },
        );
        create.write_layout(
            node,
            LayoutBox {
                x: 0.0,
                y: 0.0,
                width: 100.0,
                height: 40.0,
            },
        );
        context.commit_mutations(create).unwrap();
        let work = context.take_system_work();
        context.resolve_styles(&work.style).unwrap();
        router
            .sync_cursor_request(&context, source, device, generation, document, Some(node))
            .unwrap();
        let requests = router.take_host_service_requests(&context, 8);
        assert!(requests.iter().any(|request| {
            matches!(request, HostServiceRequest::Cursor { cursor, context } if cursor == "pointer" && context.node == Some(node.get()))
        }));
        assert!(
            !router
                .apply_host_service_response(
                    &mut context,
                    HostServiceResponse {
                        request: HostServiceRequest::Cursor {
                            context: HostRequestContext {
                                source,
                                generation,
                                document: document.get(),
                                node: Some(node.get()),
                            },
                            cursor: "pointer".into(),
                        },
                        outcome: HostServiceOutcome::Success,
                    },
                )
                .unwrap()
        );
        assert_eq!(router.counters().host_requests_stale_dropped, 0);

        router
            .sync_cursor_request(&context, source, device, generation, document, Some(node))
            .unwrap();
        assert!(
            router
                .take_host_service_requests(&context, 8)
                .iter()
                .all(|request| !matches!(request, HostServiceRequest::Cursor { .. }))
        );

        let text_input = context
            .create_component(document, nana_ui_runtime::TextInput::new("text"))
            .unwrap()
            .stable_id();
        let work = context.take_system_work();
        context.resolve_styles(&work.style).unwrap();
        router.attach(InputSourceId(22), generation, document);
        router
            .sync_cursor_request(
                &context,
                InputSourceId(22),
                nana_ui_platform::DeviceId(2),
                generation,
                document,
                Some(text_input),
            )
            .unwrap();
        let requests = router.take_host_service_requests(&context, 8);
        assert!(requests.iter().any(
            |request| matches!(request, HostServiceRequest::Cursor { cursor, .. } if cursor == "text")
        ));
    }

    #[test]
    fn attach_rejects_generation_regression_and_same_generation_rebind() {
        let mut router = InputRouter::default();
        let source = InputSourceId(19);
        let first = DocumentId::new(12).unwrap();
        let second = DocumentId::new(13).unwrap();
        assert!(router.attach(source, EndpointGeneration(5), first));
        assert!(!router.attach(source, EndpointGeneration(4), first));
        assert!(!router.attach(source, EndpointGeneration(5), second));
        assert_eq!(router.binding(source), Some((EndpointGeneration(5), first)));
        router.pointers.insert(
            (
                source,
                nana_ui_platform::DeviceId(0),
                nana_ui_platform::PointerId(1),
            ),
            PointerIdentity {
                local: 1,
                pointer_type: nana_ui_platform::PointerType::Mouse,
                is_primary: true,
            },
        );
        assert!(router.attach(source, EndpointGeneration(5), first));
        assert!(router.pointers.contains_key(&(
            source,
            nana_ui_platform::DeviceId(0),
            nana_ui_platform::PointerId(1)
        )));
        assert!(router.attach(source, EndpointGeneration(6), second));
        assert_eq!(
            router.binding(source),
            Some((EndpointGeneration(6), second))
        );
    }

    #[test]
    fn disconnect_rejects_later_device_input_until_reconnected() {
        let mut router = InputRouter::default();
        let source = InputSourceId(10);
        let device = nana_ui_platform::DeviceId(7);
        let generation = EndpointGeneration(1);
        let document = DocumentId::new(2).unwrap();
        router.attach(source, generation, document);
        let mut context = AppContext::new();
        let disconnected = event(source, generation, InputPayload::DeviceDisconnected);
        assert!(
            router
                .route(
                    &mut context,
                    &CanonicalInputEvent {
                        metadata: InputMetadata {
                            device,
                            ..disconnected.metadata
                        },
                        ..disconnected
                    },
                    Duration::ZERO,
                    None
                )
                .is_ok()
        );
        let stale = event(source, generation, InputPayload::Focus { focused: true });
        let stale = CanonicalInputEvent {
            metadata: InputMetadata {
                device,
                ..stale.metadata
            },
            ..stale
        };
        assert_eq!(
            router.route(&mut context, &stale, Duration::ZERO, None),
            Err(InputRouterError::Disconnected)
        );
        let connected = CanonicalInputEvent {
            metadata: InputMetadata {
                device,
                sequence: InputSequence(2),
                ..stale.metadata
            },
            payload: InputPayload::DeviceConnected,
        };
        assert!(
            router
                .route(&mut context, &connected, Duration::ZERO, None)
                .is_ok()
        );
        assert!(
            router
                .route(
                    &mut context,
                    &CanonicalInputEvent {
                        metadata: InputMetadata {
                            sequence: InputSequence(3),
                            ..connected.metadata
                        },
                        payload: InputPayload::Focus { focused: true },
                    },
                    Duration::ZERO,
                    None,
                )
                .is_ok()
        );
    }

    #[test]
    fn source_disconnect_drops_focus_owner_state() {
        let mut router = InputRouter::default();
        let source = InputSourceId(12);
        let generation = EndpointGeneration(1);
        let document = DocumentId::new(4).unwrap();
        router.attach(source, generation, document);
        let mut context = AppContext::new();
        router
            .route(
                &mut context,
                &event(source, generation, InputPayload::Focus { focused: true }),
                Duration::ZERO,
                None,
            )
            .unwrap();
        assert!(router.focused_sources.contains_key(&document));
        router
            .route(
                &mut context,
                &CanonicalInputEvent {
                    metadata: InputMetadata {
                        source,
                        device: nana_ui_platform::DeviceId(0),
                        generation,
                        sequence: InputSequence(2),
                        timestamp: InputTimestamp(2),
                    },
                    payload: InputPayload::SourceDisconnected,
                },
                Duration::ZERO,
                None,
            )
            .unwrap();
        assert!(!router.focused_sources.contains_key(&document));
    }

    #[test]
    fn disconnect_dispatches_cancel_and_revokes_runtime_capture() {
        let mut router = InputRouter::default();
        let source = InputSourceId(15);
        let device = nana_ui_platform::DeviceId(3);
        let generation = EndpointGeneration(1);
        let document = DocumentId::new(7).unwrap();
        router.attach(source, generation, document);
        let mut context = AppContext::new();
        let node = StableNodeId::new(1).unwrap();
        let mut create = MutationQueue::new();
        create.create(node, document, NodeKind::Element { tag: "div".into() });
        create.set_interaction(
            node,
            InteractionState {
                pointer_events: true,
                ..InteractionState::default()
            },
        );
        context.commit_mutations(create).unwrap();
        context.rebuild_hit_test(document);
        let mut capture = MutationQueue::new();
        capture.capture_pointer(1, node);
        context.commit_mutations(capture).unwrap();
        context.set_pointer_location(document, 1, Some((10.0, 12.0)));
        router
            .route(
                &mut context,
                &pointer_event(source, generation, device, 1, PointerPhase::Down),
                Duration::ZERO,
                None,
            )
            .unwrap();
        assert_eq!(context.world().pointer_capture(document, 1), Some(node));
        let captured_work_revision = context.world().pending_work_revision();
        let snapshot = router.last_route_snapshot().unwrap();
        assert_eq!(snapshot.source, source);
        assert_eq!(snapshot.device, device);
        assert_eq!(snapshot.kind, CanonicalInputKind::Pointer);
        assert_eq!(snapshot.capture_owner, Some(node));
        for sequence in 2..=1001 {
            router
                .route(
                    &mut context,
                    &pointer_event(source, generation, device, sequence, PointerPhase::Move),
                    Duration::ZERO,
                    None,
                )
                .unwrap();
        }
        let counters = router.counters();
        assert_eq!(counters.hit_tests, 1, "only the initial down may hit-test");
        assert_eq!(counters.routing_cache_hits, 1000);
        assert_eq!(
            context.world().pending_work_revision(),
            captured_work_revision,
            "captured pointer moves must not schedule unrelated Runtime work"
        );
        router
            .route(
                &mut context,
                &CanonicalInputEvent {
                    metadata: InputMetadata {
                        source,
                        device,
                        generation,
                        sequence: InputSequence(1002),
                        timestamp: InputTimestamp(1002),
                    },
                    payload: InputPayload::Focus { focused: true },
                },
                Duration::ZERO,
                None,
            )
            .unwrap();
        let disconnected = CanonicalInputEvent {
            metadata: InputMetadata {
                source,
                device,
                generation,
                sequence: InputSequence(1003),
                timestamp: InputTimestamp(1003),
            },
            payload: InputPayload::DeviceDisconnected,
        };
        router
            .route(&mut context, &disconnected, Duration::ZERO, None)
            .unwrap();
        assert_eq!(context.world().pointer_capture(document, 1), None);
        assert!(router.pointers.is_empty());
    }

    #[test]
    fn pointer_leave_clears_hover_without_releasing_capture() {
        let mut router = InputRouter::default();
        let source = InputSourceId(17);
        let device = nana_ui_platform::DeviceId(5);
        let generation = EndpointGeneration(1);
        let document = DocumentId::new(17).unwrap();
        router.attach(source, generation, document);
        let mut context = AppContext::new();
        let node = StableNodeId::new(4).unwrap();
        let mut create = MutationQueue::new();
        create.create(node, document, NodeKind::Element { tag: "div".into() });
        create.set_interaction(
            node,
            InteractionState {
                pointer_events: true,
                ..InteractionState::default()
            },
        );
        context.commit_mutations(create).unwrap();
        context.rebuild_hit_test(document);
        let mut capture = MutationQueue::new();
        capture.capture_pointer(1, node);
        context.commit_mutations(capture).unwrap();
        context.set_pointer_location(document, 1, Some((10.0, 12.0)));
        router
            .route(
                &mut context,
                &pointer_event(source, generation, device, 1, PointerPhase::Down),
                Duration::ZERO,
                None,
            )
            .unwrap();
        assert_eq!(context.world().pointer_capture(document, 1), Some(node));

        router
            .route(
                &mut context,
                &CanonicalInputEvent {
                    metadata: InputMetadata {
                        source,
                        device,
                        generation,
                        sequence: InputSequence(2),
                        timestamp: InputTimestamp(2),
                    },
                    payload: InputPayload::PointerLeave {
                        pointer_id: nana_ui_platform::PointerId(42),
                    },
                },
                Duration::ZERO,
                None,
            )
            .unwrap();
        assert_eq!(context.world().pointer_hover(document, 1), None);
        assert_eq!(context.world().pointer_capture(document, 1), Some(node));
    }

    #[test]
    fn focus_revoke_dispatches_cancel_and_revokes_runtime_capture() {
        let mut router = InputRouter::default();
        let source = InputSourceId(16);
        let device = nana_ui_platform::DeviceId(4);
        let generation = EndpointGeneration(1);
        let document = DocumentId::new(8).unwrap();
        router.attach(source, generation, document);
        let mut context = AppContext::new();
        let node = StableNodeId::new(2).unwrap();
        let mut create = MutationQueue::new();
        create.create(node, document, NodeKind::Element { tag: "div".into() });
        create.set_interaction(
            node,
            InteractionState {
                pointer_events: true,
                ..InteractionState::default()
            },
        );
        context.commit_mutations(create).unwrap();
        context.rebuild_hit_test(document);
        let mut capture = MutationQueue::new();
        capture.capture_pointer(1, node);
        context.commit_mutations(capture).unwrap();
        context.set_pointer_location(document, 1, Some((10.0, 12.0)));
        router
            .route(
                &mut context,
                &pointer_event(source, generation, device, 1, PointerPhase::Down),
                Duration::ZERO,
                None,
            )
            .unwrap();
        assert_eq!(context.world().pointer_capture(document, 1), Some(node));
        let focus_revoke = CanonicalInputEvent {
            metadata: InputMetadata {
                source,
                device,
                generation,
                sequence: InputSequence(2),
                timestamp: InputTimestamp(2),
            },
            payload: InputPayload::Focus { focused: false },
        };
        router
            .route(&mut context, &focus_revoke, Duration::ZERO, None)
            .unwrap();
        assert_eq!(context.world().pointer_capture(document, 1), None);
        assert!(router.pointers.is_empty());
    }

    #[test]
    fn focus_revoke_preserves_other_sources_pointer_state() {
        let mut router = InputRouter::default();
        let source = InputSourceId(20);
        let other = InputSourceId(21);
        let generation = EndpointGeneration(1);
        let device = nana_ui_platform::DeviceId(1);
        let document = DocumentId::new(9).unwrap();
        router.attach(source, generation, document);
        router.attach(other, generation, document);
        let mut context = AppContext::new();
        let node = StableNodeId::new(3).unwrap();
        let mut create = MutationQueue::new();
        create.create(node, document, NodeKind::Element { tag: "div".into() });
        context.commit_mutations(create).unwrap();
        for owner in [source, other] {
            router
                .route(
                    &mut context,
                    &pointer_event(owner, generation, device, 1, PointerPhase::Move),
                    Duration::ZERO,
                    None,
                )
                .unwrap();
        }
        let other_pointer =
            router.pointers[&(other, device, nana_ui_platform::PointerId(42))].local;
        context
            .press_pointer(document, other_pointer, node)
            .unwrap();
        context
            .set_pointer_hover(document, other_pointer, Some(node))
            .unwrap();
        let mut capture = MutationQueue::new();
        capture.capture_pointer(other_pointer, node);
        context.commit_mutations(capture).unwrap();
        router
            .route(
                &mut context,
                &CanonicalInputEvent {
                    metadata: InputMetadata {
                        source,
                        device,
                        generation,
                        sequence: InputSequence(2),
                        timestamp: InputTimestamp(2),
                    },
                    payload: InputPayload::Focus { focused: false },
                },
                Duration::ZERO,
                None,
            )
            .unwrap();
        assert_eq!(
            context.world().pointer_press(document, other_pointer),
            Some(node)
        );
        assert_eq!(
            context.world().pointer_hover(document, other_pointer),
            Some(node)
        );
        assert_eq!(
            context.world().pointer_capture(document, other_pointer),
            Some(node)
        );
        assert_eq!(router.pointers.len(), 1);
    }

    #[test]
    fn mouse_pointer_identity_survives_button_release_for_hover_continuity() {
        let mut router = InputRouter::default();
        let source = InputSourceId(11);
        let generation = EndpointGeneration(1);
        let device = nana_ui_platform::DeviceId(2);
        let document = DocumentId::new(3).unwrap();
        router.attach(source, generation, document);
        let pointer = nana_ui_platform::PointerInput {
            phase: PointerPhase::Down,
            pointer_id: nana_ui_platform::PointerId(42),
            pointer_type: nana_ui_platform::PointerType::Mouse,
            x: 0.0,
            y: 0.0,
            screen_x: 0.0,
            screen_y: 0.0,
            button: 0,
            buttons: 1,
            pressure: 0.5,
            tangential_pressure: 0.0,
            tilt_x: 0,
            tilt_y: 0,
            twist: 0,
            is_primary: true,
            activation_click: false,
            modifiers: Default::default(),
        };
        let down = CanonicalInputEvent {
            metadata: InputMetadata {
                source,
                device,
                generation,
                sequence: InputSequence(1),
                timestamp: InputTimestamp(1),
            },
            payload: InputPayload::Pointer(pointer),
        };
        let up = CanonicalInputEvent {
            metadata: InputMetadata {
                sequence: InputSequence(2),
                timestamp: InputTimestamp(2),
                ..down.metadata
            },
            payload: InputPayload::Pointer(nana_ui_platform::PointerInput {
                phase: PointerPhase::Up,
                buttons: 0,
                ..pointer
            }),
        };
        let mut context = AppContext::new();
        let _ = router.route(&mut context, &down, Duration::ZERO, None);
        assert_eq!(router.pointers.len(), 1);
        let _ = router.route(&mut context, &up, Duration::ZERO, None);
        assert_eq!(router.pointers.len(), 1);

        let touch_down = CanonicalInputEvent {
            metadata: InputMetadata {
                sequence: InputSequence(3),
                timestamp: InputTimestamp(3),
                ..down.metadata
            },
            payload: InputPayload::Pointer(nana_ui_platform::PointerInput {
                pointer_type: nana_ui_platform::PointerType::Touch,
                pointer_id: nana_ui_platform::PointerId(43),
                ..pointer
            }),
        };
        let touch_up = CanonicalInputEvent {
            metadata: InputMetadata {
                sequence: InputSequence(4),
                timestamp: InputTimestamp(4),
                ..down.metadata
            },
            payload: InputPayload::Pointer(nana_ui_platform::PointerInput {
                phase: PointerPhase::Up,
                pointer_type: nana_ui_platform::PointerType::Touch,
                pointer_id: nana_ui_platform::PointerId(43),
                buttons: 0,
                ..pointer
            }),
        };
        let _ = router.route(&mut context, &touch_down, Duration::ZERO, None);
        let _ = router.route(&mut context, &touch_up, Duration::ZERO, None);
        assert_eq!(router.pointers.len(), 1);
    }

    #[test]
    fn ime_lifecycle_emits_bounded_host_service_intents() {
        let mut router = InputRouter::default();
        let source = InputSourceId(12);
        let generation = EndpointGeneration(4);
        let document = DocumentId::new(4).unwrap();
        router.attach(source, generation, document);
        let mut context = AppContext::new();
        let input = context
            .create_component(document, nana_ui_runtime::TextInput::new(""))
            .unwrap();
        context.focus_node(document, input.stable_id()).unwrap();
        for (sequence, composition) in [
            (1, CompositionInput::Enabled),
            (
                2,
                CompositionInput::Update {
                    text: "候选".into(),
                    selection: Some((0, 3)),
                },
            ),
            (3, CompositionInput::Disabled),
        ] {
            let event = CanonicalInputEvent {
                metadata: InputMetadata {
                    source,
                    device: nana_ui_platform::DeviceId(1),
                    generation,
                    sequence: InputSequence(sequence),
                    timestamp: InputTimestamp(sequence),
                },
                payload: InputPayload::Composition(composition),
            };
            router
                .route(&mut context, &event, Duration::ZERO, None)
                .expect("IME event routes");
        }
        let requests = router.take_host_service_requests(&context, 8);
        assert!(matches!(requests[0], HostServiceRequest::ImeEnable { .. }));
        assert!(matches!(
            requests[1],
            HostServiceRequest::ImeUpdate {
                surrounding: Some(_),
                ..
            }
        ));
        assert!(matches!(requests[2], HostServiceRequest::ImeDisable { .. }));
        assert!(router.take_host_service_requests(&context, 8).is_empty());
    }

    #[test]
    fn endpoint_generation_rebind_rearms_ime_enable_for_same_document() {
        let mut router = InputRouter::default();
        let source = InputSourceId(26);
        let document = DocumentId::new(20).unwrap();
        let mut context = AppContext::new();
        let input = context
            .create_component(document, nana_ui_runtime::TextInput::new("text"))
            .unwrap();
        context.focus_node(document, input.stable_id()).unwrap();

        router.attach(source, EndpointGeneration(1), document);
        router
            .route(
                &mut context,
                &CanonicalInputEvent {
                    metadata: InputMetadata {
                        source,
                        device: nana_ui_platform::DeviceId(1),
                        generation: EndpointGeneration(1),
                        sequence: InputSequence(1),
                        timestamp: InputTimestamp(1),
                    },
                    payload: InputPayload::Composition(CompositionInput::Enabled),
                },
                Duration::ZERO,
                None,
            )
            .unwrap();
        let first = router.take_host_service_requests(&context, 8);
        assert!(
            first
                .iter()
                .any(|request| matches!(request, HostServiceRequest::ImeEnable { .. }))
        );

        assert!(router.attach(source, EndpointGeneration(2), document));
        router
            .route(
                &mut context,
                &CanonicalInputEvent {
                    metadata: InputMetadata {
                        source,
                        device: nana_ui_platform::DeviceId(1),
                        generation: EndpointGeneration(2),
                        sequence: InputSequence(1),
                        timestamp: InputTimestamp(1),
                    },
                    payload: InputPayload::Composition(CompositionInput::Enabled),
                },
                Duration::ZERO,
                None,
            )
            .unwrap();
        let rebound = router.take_host_service_requests(&context, 8);
        assert!(
            rebound
                .iter()
                .any(|request| matches!(request, HostServiceRequest::ImeEnable { .. }))
        );
    }

    #[test]
    fn ime_update_without_focus_owner_is_dropped_at_host_boundary() {
        let mut router = InputRouter::default();
        let source = InputSourceId(24);
        let generation = EndpointGeneration(1);
        let document = DocumentId::new(18).unwrap();
        router.attach(source, generation, document);
        let mut context = AppContext::new();
        context
            .create_component(document, nana_ui_runtime::TextInput::new("text"))
            .unwrap();
        router
            .route(
                &mut context,
                &event(
                    source,
                    generation,
                    InputPayload::Composition(CompositionInput::Update {
                        text: "stale".into(),
                        selection: None,
                    }),
                ),
                Duration::ZERO,
                None,
            )
            .unwrap();
        assert!(router.take_host_service_requests(&context, 8).is_empty());
        assert_eq!(router.counters().host_requests_stale_dropped, 1);
    }

    #[test]
    fn focus_lifecycle_emits_native_text_input_capability_intents() {
        let mut router = InputRouter::default();
        let source = InputSourceId(25);
        let generation = EndpointGeneration(1);
        let document = DocumentId::new(19).unwrap();
        router.attach(source, generation, document);
        let mut context = AppContext::new();
        let input = context
            .create_component(document, nana_ui_runtime::TextInput::new("text"))
            .unwrap();
        context.focus_node(document, input.stable_id()).unwrap();
        router
            .route(
                &mut context,
                &event(source, generation, InputPayload::Focus { focused: true }),
                Duration::ZERO,
                None,
            )
            .unwrap();
        let enabled = router.take_host_service_requests(&context, 8);
        assert!(enabled.iter().any(|request| {
            matches!(
                request,
                HostServiceRequest::NativeTextInput {
                    enabled: true,
                    context,
                } if context.node == Some(input.stable_id().get())
            )
        }));

        context.clear_focus(document).unwrap();
        router
            .route(
                &mut context,
                &CanonicalInputEvent {
                    metadata: InputMetadata {
                        sequence: InputSequence(2),
                        timestamp: InputTimestamp(2),
                        ..event(source, generation, InputPayload::Focus { focused: false }).metadata
                    },
                    payload: InputPayload::Focus { focused: false },
                },
                Duration::ZERO,
                None,
            )
            .unwrap();
        let disabled = router.take_host_service_requests(&context, 8);
        assert!(disabled.iter().any(|request| {
            matches!(
                request,
                HostServiceRequest::NativeTextInput {
                    enabled: false,
                    context,
                } if context.node.is_none()
            )
        }));
    }

    #[test]
    fn ime_enable_carries_focused_caret_anchor_in_logical_coordinates() {
        let mut router = InputRouter::default();
        let source = InputSourceId(23);
        let generation = EndpointGeneration(1);
        let document = DocumentId::new(16).unwrap();
        router.attach(source, generation, document);
        let mut context = AppContext::new();
        let input = context
            .create_component(document, nana_ui_runtime::TextInput::new("text"))
            .unwrap();
        let mut layout = MutationQueue::new();
        layout.write_layout(
            input.stable_id(),
            LayoutBox {
                x: 12.0,
                y: 24.0,
                width: 180.0,
                height: 28.0,
            },
        );
        context.commit_mutations(layout).unwrap();
        assert!(context.focus_node(document, input.stable_id()).unwrap());
        router
            .route(
                &mut context,
                &event(source, generation, InputPayload::Focus { focused: true }),
                Duration::ZERO,
                None,
            )
            .unwrap();
        let requests = router.take_host_service_requests(&context, 8);
        let HostServiceRequest::ImeEnable {
            surrounding: Some(surrounding),
            ..
        } = &requests[0]
        else {
            panic!("focused IME must carry surrounding text and caret anchor");
        };
        let anchor = surrounding.cursor_area.expect("caret anchor");
        assert_eq!(
            (anchor.x, anchor.y, anchor.width, anchor.height),
            (12.0, 24.0, 180.0, 28.0)
        );
    }

    #[test]
    fn endpoint_retains_event_when_host_service_backpressure_blocks_route() {
        let mut router = InputRouter {
            host_requests: HostServiceQueue::new(1),
            ..InputRouter::default()
        };
        let source = InputSourceId(14);
        let generation = EndpointGeneration(1);
        let document = DocumentId::new(6).unwrap();
        router.attach(source, generation, document);
        let mut context = AppContext::new();
        let input = context
            .create_component(document, nana_ui_runtime::TextInput::new(""))
            .unwrap();
        context.focus_node(document, input.stable_id()).unwrap();
        let mut endpoint = nana_ui_platform::InputEndpoint::new(source, generation, 4, 4096);
        for (sequence, composition) in [
            (1, CompositionInput::Enabled),
            (
                2,
                CompositionInput::Update {
                    text: "候选".into(),
                    selection: Some((0, 3)),
                },
            ),
        ] {
            endpoint
                .push(CanonicalInputEvent {
                    metadata: InputMetadata {
                        source,
                        device: nana_ui_platform::DeviceId(1),
                        generation,
                        sequence: InputSequence(sequence),
                        timestamp: InputTimestamp(sequence),
                    },
                    payload: InputPayload::Composition(composition),
                })
                .unwrap();
        }

        assert_eq!(
            router.route_endpoint(&mut context, &mut endpoint, Duration::ZERO),
            Err(InputRouterError::HostServiceBackpressure)
        );
        assert_eq!(endpoint.len(), 1);
        router.take_host_service_requests(&context, 1);
        assert_eq!(
            router.route_endpoint(&mut context, &mut endpoint, Duration::ZERO),
            Ok(1)
        );
        assert!(endpoint.is_empty());
    }

    #[test]
    fn detached_endpoint_drops_queued_host_intents_before_host_drain() {
        let mut router = InputRouter::default();
        let source = InputSourceId(13);
        let generation = EndpointGeneration(1);
        let document = DocumentId::new(5).unwrap();
        router.attach(source, generation, document);
        let event = event(
            source,
            generation,
            InputPayload::Composition(CompositionInput::Enabled),
        );
        let mut context = AppContext::new();
        context
            .create_component(document, nana_ui_runtime::TextInput::new(""))
            .unwrap();
        router
            .route(&mut context, &event, Duration::ZERO, None)
            .expect("IME event routes");
        router.detach(source);
        assert!(router.take_host_service_requests(&context, 8).is_empty());
        assert_eq!(router.counters().host_requests_stale_dropped, 1);
    }

    #[test]
    fn host_request_for_unmounted_document_is_dropped() {
        let mut router = InputRouter::default();
        let source = InputSourceId(15);
        let generation = EndpointGeneration(1);
        let document = DocumentId::new(7).unwrap();
        router.attach(source, generation, document);
        let mut context = AppContext::new();
        router
            .route(
                &mut context,
                &event(
                    source,
                    generation,
                    InputPayload::Composition(CompositionInput::Enabled),
                ),
                Duration::ZERO,
                None,
            )
            .unwrap();
        let mut services = nana_ui_platform::UnsupportedHostServices;
        assert!(
            router
                .service_host_requests(&context, &mut services, 8)
                .is_empty()
        );
        assert_eq!(router.counters().host_requests_stale_dropped, 1);
    }

    #[test]
    fn detach_invalidates_last_route_snapshot() {
        let mut router = InputRouter::default();
        let source = InputSourceId(18);
        let generation = EndpointGeneration(1);
        let document = DocumentId::new(11).unwrap();
        router.attach(source, generation, document);
        let mut context = AppContext::new();
        router
            .route(
                &mut context,
                &event(source, generation, InputPayload::Focus { focused: true }),
                Duration::ZERO,
                None,
            )
            .unwrap();
        assert!(router.last_route_snapshot().is_some());
        assert_eq!(router.detach(source), Some(document));
        assert!(router.last_route_snapshot().is_none());
    }

    #[test]
    fn host_service_boundary_returns_capability_outcomes_without_routing_work() {
        let mut router = InputRouter::default();
        let source = InputSourceId(14);
        let generation = EndpointGeneration(1);
        let document = DocumentId::new(6).unwrap();
        router.attach(source, generation, document);
        let event = event(
            source,
            generation,
            InputPayload::Composition(CompositionInput::Enabled),
        );
        let mut context = AppContext::new();
        let input = context
            .create_component(document, nana_ui_runtime::TextInput::new(""))
            .unwrap();
        context.focus_node(document, input.stable_id()).unwrap();
        router
            .route(&mut context, &event, Duration::ZERO, None)
            .expect("IME event routes");
        let mut services = nana_ui_platform::UnsupportedHostServices;
        let responses = router.service_host_requests_with_results(&context, &mut services, 8);
        assert_eq!(responses.len(), 1);
        assert_eq!(
            responses[0].outcome,
            nana_ui_platform::HostServiceOutcome::Unsupported
        );
        assert_eq!(responses[0].request.context().document, document.get());
        assert!(router.take_host_service_requests(&context, 8).is_empty());
    }

    #[test]
    fn clipboard_response_applies_only_to_a_live_bound_document() {
        let mut router = InputRouter::default();
        let source = InputSourceId(19);
        let generation = EndpointGeneration(1);
        let document = DocumentId::new(12).unwrap();
        router.attach(source, generation, document);
        let mut context = AppContext::new();
        let input = context
            .create_component(document, nana_ui_runtime::TextInput::new("before"))
            .unwrap();
        assert!(context.focus_node(document, input.stable_id()).unwrap());

        let response = HostServiceResponse {
            request: HostServiceRequest::ClipboardRead {
                context: HostRequestContext {
                    source,
                    generation,
                    document: document.get(),
                    node: Some(input.stable_id().get()),
                },
            },
            outcome: HostServiceOutcome::ClipboardText(Some("after".into())),
        };
        assert!(
            router
                .apply_host_service_response(&mut context, response)
                .unwrap()
        );
        assert_eq!(context.world().text(input.stable_id()), Some("beforeafter"));

        router.detach(source);
        let stale = HostServiceResponse {
            request: HostServiceRequest::ClipboardRead {
                context: HostRequestContext {
                    source,
                    generation,
                    document: document.get(),
                    node: Some(input.stable_id().get()),
                },
            },
            outcome: HostServiceOutcome::ClipboardText(Some("stale".into())),
        };
        assert!(
            !router
                .apply_host_service_response(&mut context, stale)
                .unwrap()
        );
        assert_eq!(context.world().text(input.stable_id()), Some("beforeafter"));
    }

    #[test]
    fn canonical_clipboard_shortcuts_are_host_requests_and_cut_is_atomic() {
        use std::borrow::Cow;

        let mut context = AppContext::new();
        let document = DocumentId::new(13).unwrap();
        let input = context
            .create_component(document, nana_ui_runtime::TextInput::new("copied"))
            .unwrap()
            .stable_id();
        context.focus_node(document, input).unwrap();
        context.select_all_focused_text(document).unwrap();
        let source = InputSourceId(20);
        let generation = EndpointGeneration(1);
        let mut router = InputRouter::default();
        router.attach(source, generation, document);
        let key = |sequence, logical| CanonicalInputEvent {
            metadata: InputMetadata {
                source,
                device: nana_ui_platform::DeviceId(1),
                generation,
                sequence: InputSequence(sequence),
                timestamp: InputTimestamp(sequence),
            },
            payload: InputPayload::Key(nana_ui_platform::KeyInput {
                physical: nana_ui_platform::PhysicalKey(Cow::Borrowed("KeyC")),
                logical: nana_ui_platform::LogicalKey(Cow::Borrowed(logical)),
                state: nana_ui_platform::KeyState::Pressed,
                repeat: false,
                modifiers: nana_ui_platform::InputModifiers {
                    control: true,
                    ..Default::default()
                },
            }),
        };

        assert!(
            router
                .route(&mut context, &key(1, "c"), Duration::ZERO, None)
                .unwrap()
                .handled
        );
        let mut host = nana_ui_platform::ClipboardHostServices::new(
            nana_ui_platform::shared_clipboard(nana_ui_platform::MemoryClipboard::new()),
        );
        let responses = router.service_host_requests_with_results(&context, &mut host, 8);
        assert!(matches!(
            responses[0].request,
            HostServiceRequest::ClipboardWrite { cut: false, .. }
        ));
        assert!(
            !router
                .apply_host_service_response(&mut context, responses.into_iter().next().unwrap())
                .unwrap()
        );
        assert_eq!(context.world().text(input), Some("copied"));

        assert!(
            router
                .route(&mut context, &key(2, "x"), Duration::ZERO, None)
                .unwrap()
                .handled
        );
        let responses = router.service_host_requests_with_results(&context, &mut host, 8);
        assert!(
            router
                .apply_host_service_response(&mut context, responses.into_iter().next().unwrap())
                .unwrap()
        );
        assert_eq!(context.world().text(input), Some(""));
    }

    #[test]
    fn document_selection_copy_does_not_require_editor_focus() {
        use std::borrow::Cow;

        let mut context = AppContext::new();
        let document = DocumentId::new(21).unwrap();
        let node = context
            .create_component(document, nana_ui_runtime::TextInput::new("selected"))
            .unwrap()
            .stable_id();
        context.compat_world_mut().set_document_text_selection(
            document,
            Some(nana_ui_runtime::DocumentTextSelection {
                node,
                start: 0,
                end: 8,
                lines: Vec::new(),
            }),
        );
        let source = InputSourceId(27);
        let generation = EndpointGeneration(1);
        let mut router = InputRouter::default();
        router.attach(source, generation, document);
        let event = CanonicalInputEvent {
            metadata: InputMetadata {
                source,
                device: nana_ui_platform::DeviceId(1),
                generation,
                sequence: InputSequence(1),
                timestamp: InputTimestamp(1),
            },
            payload: InputPayload::Key(nana_ui_platform::KeyInput {
                physical: nana_ui_platform::PhysicalKey(Cow::Borrowed("KeyC")),
                logical: nana_ui_platform::LogicalKey(Cow::Borrowed("c")),
                state: nana_ui_platform::KeyState::Pressed,
                repeat: false,
                modifiers: nana_ui_platform::InputModifiers {
                    control: true,
                    ..Default::default()
                },
            }),
        };
        router
            .route(&mut context, &event, Duration::ZERO, None)
            .unwrap();
        context.clear_focus(document).unwrap();
        let request = router
            .take_host_service_requests(&context, 1)
            .into_iter()
            .next()
            .expect("document copy request");
        assert!(matches!(
            request,
            HostServiceRequest::ClipboardWrite { cut: false, .. }
        ));
        assert!(
            !router
                .apply_host_service_response(
                    &mut context,
                    HostServiceResponse {
                        request,
                        outcome: HostServiceOutcome::Success,
                    },
                )
                .unwrap()
        );
        assert_eq!(router.counters().host_requests_stale_dropped, 0);
    }

    #[test]
    fn unsupported_capability_is_not_invoked_at_host_boundary() {
        struct NoImeHost {
            calls: usize,
        }
        impl HostServices for NoImeHost {
            fn supports(&self, capability: nana_ui_platform::HostCapability) -> bool {
                capability != nana_ui_platform::HostCapability::Ime
            }
            fn request(&mut self, _request: HostServiceRequest) -> HostServiceOutcome {
                self.calls += 1;
                HostServiceOutcome::Failed("unsupported request reached host".into())
            }
        }

        let mut router = InputRouter::default();
        let source = InputSourceId(17);
        let generation = EndpointGeneration(1);
        let document = DocumentId::new(10).unwrap();
        router.attach(source, generation, document);
        let mut context = AppContext::new();
        let input = context
            .create_component(document, nana_ui_runtime::TextInput::new(""))
            .unwrap();
        context.focus_node(document, input.stable_id()).unwrap();
        router
            .route(
                &mut context,
                &event(
                    source,
                    generation,
                    InputPayload::Composition(CompositionInput::Enabled),
                ),
                Duration::ZERO,
                None,
            )
            .unwrap();
        let mut host = NoImeHost { calls: 0 };
        assert_eq!(
            router.service_host_requests(&context, &mut host, 8),
            vec![HostServiceOutcome::Unsupported]
        );
        assert_eq!(host.calls, 0);
    }
}

#[cfg(test)]
mod hover_card_tests;
