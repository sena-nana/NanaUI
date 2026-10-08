//! Canonical input routing: the one path from a host's input events into the
//! retained world.
//!
//! A host binds each input source (a window, a headless session) to the
//! document it drives, then hands the context that source's events, one at a
//! time through [`AppContext::route_input`] or as a batch through
//! [`AppContext::drain_input`]. Routing validates the event against the
//! binding, resolves its pointer to a context-local id, applies it, and keeps
//! the host's cursor and text-input state in step through [`HostServices`].
//! All routing state lives here, on the context the source is bound to, so
//! two windows never share a pointer, an IME owner or a cursor.

mod dispatch;
mod effects;
mod headless;

use std::collections::{HashMap, HashSet};
use std::time::{Duration, Instant};

use nana_ui_input::{
    CanonicalInputEvent, CompositionInput, DeviceId, EndpointGeneration, HostServices,
    InputDisposition, InputEndpoint, InputPayload, InputSequence, InputSourceId, InputTimestamp,
    KeyState, PointerId, PointerInput, PointerPhase, PointerType, WheelInput,
};

use super::OverlayActivity;
use crate::{AppContext, DocumentId, FrameworkError, StableNodeId, TextShaper};

pub(crate) use dispatch::LINE_SCROLL_EXTENT;
use dispatch::{KeyStroke, reborrow_text_shaper};
pub use headless::HeadlessInput;

/// What routing one event did.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct InputRouteOutcome {
    /// A control acted on the event.
    pub handled: bool,
    /// The host must not apply its own default for the event.
    pub prevent_default: bool,
    /// For pointer and wheel events: the node holding the pointer's capture,
    /// else the topmost node under it that the event could reach. For a
    /// file drag: the drop target it is over or landed on.
    pub pointer_hit: Option<StableNodeId>,
    /// The event scheduled Runtime work: something must be drawn again.
    pub invalidated_work: bool,
    /// For pointer, wheel, enter and leave events: the context's id for the
    /// pointer, the one capture, hover and press are keyed by. A platform id
    /// maps to a new one after a blur or on another window of the context.
    pub pointer_id: Option<u64>,
}

impl InputRouteOutcome {
    pub fn disposition(&self) -> InputDisposition {
        InputDisposition {
            handled: self.handled,
            prevent_default: self.prevent_default,
        }
    }

    /// `event` as observers of this context see it: about the context's
    /// pointer id, so a page that captures the pointer it was told about
    /// captures the one the router follows.
    pub fn localize(&self, event: CanonicalInputEvent) -> CanonicalInputEvent {
        match self.pointer_id {
            Some(local) => event.with_pointer_id(PointerId(local)),
            None => event,
        }
    }
}

/// Why an event was not routed.
#[derive(Debug)]
pub enum InputRouteError {
    /// No document is bound to the event's source.
    UnknownSource,
    /// The event belongs to an earlier binding of its source.
    StaleGeneration,
    /// Its source or device is disconnected.
    Disconnected,
    /// Its sequence does not follow the source's last event.
    OutOfOrder,
    /// Its timestamp is earlier than the source's last event.
    TimestampRegression,
    /// The world rejected what the event asked of it.
    Dispatch(FrameworkError),
}

impl std::fmt::Display for InputRouteError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::UnknownSource => f.write_str("input source is not bound to a document"),
            Self::StaleGeneration => f.write_str("input endpoint generation is stale"),
            Self::Disconnected => f.write_str("input source or device is disconnected"),
            Self::OutOfOrder => f.write_str("input sequence is out of order"),
            Self::TimestampRegression => f.write_str("input timestamp regressed"),
            Self::Dispatch(error) => write!(f, "input dispatch failed: {error}"),
        }
    }
}

impl std::error::Error for InputRouteError {}

impl From<FrameworkError> for InputRouteError {
    fn from(error: FrameworkError) -> Self {
        Self::Dispatch(error)
    }
}

/// Why a source could not be bound.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InputBindError {
    /// The source is bound at a later generation already.
    StaleGeneration { bound: EndpointGeneration },
    /// The source is bound to another document at this generation; unbind it
    /// or advance the generation first.
    DocumentRebind { bound: DocumentId },
}

impl std::fmt::Display for InputBindError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::StaleGeneration { bound } => {
                write!(f, "input source is already bound at generation {}", bound.0)
            }
            Self::DocumentRebind { bound } => write!(
                f,
                "input source is bound to document {} at this generation",
                bound.get()
            ),
        }
    }
}

impl std::error::Error for InputBindError {}

/// One drained event and what routing it did.
#[derive(Debug)]
pub struct RoutedEvent {
    pub event: CanonicalInputEvent,
    pub result: Result<InputRouteOutcome, InputRouteError>,
}

/// Work input routing did on this context, cumulative.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct InputCounters {
    pub events_routed: u64,
    pub events_rejected: u64,
    pub hover_changes: u64,
    pub cursor_updates: u64,
    pub text_input_updates: u64,
    /// Committed text dropped because the key that produced it was handled.
    pub text_suppressed: u64,
}

/// A pointer as the context knows it: one local id per source, device and
/// platform pointer, so two sources never share capture or hover.
#[derive(Debug, Clone, Copy)]
struct PointerIdentity {
    local: u64,
    pointer_type: PointerType,
    is_primary: bool,
}

/// What routing remembers about one bound source.
#[derive(Debug)]
struct SourceState {
    generation: EndpointGeneration,
    document: DocumentId,
    last: Option<(InputSequence, InputTimestamp)>,
    disconnected: bool,
    disconnected_devices: HashSet<DeviceId>,
    pointers: HashMap<(DeviceId, PointerId), PointerIdentity>,
    /// Whether the host window this source stands for has focus.
    focused: bool,
    /// Whether this source has ever held the document focus. Window blur
    /// deliberately keeps the document's focused control, so a later source
    /// disconnect must still be able to clear that focus even though
    /// `focused` is already false.
    focus_ever: bool,
    /// The last pointer position on this source, for re-deriving the cursor
    /// after the world under it changed.
    pointer: Option<(u64, f32, f32)>,
    effects: effects::SourceEffects,
    /// The last key press handled by Runtime; text naming it is dropped.
    handled_key: Option<InputSequence>,
    /// A file drag from this source is hovering the document.
    file_drag: bool,
}

impl SourceState {
    fn new(generation: EndpointGeneration, document: DocumentId) -> Self {
        Self {
            generation,
            document,
            last: None,
            disconnected: false,
            disconnected_devices: HashSet::new(),
            pointers: HashMap::new(),
            focused: false,
            focus_ever: false,
            pointer: None,
            effects: effects::SourceEffects::default(),
            handled_key: None,
            file_drag: false,
        }
    }
}

/// The key a stroke names. With Control or Command held on a layout whose
/// letters are not Latin (Cyrillic, Greek, Hebrew), the logical key is that
/// layout's letter and no shortcut would ever match; the key in the same
/// place on a Latin keyboard is the one shortcuts are written for.
fn shortcut_key(key: &nana_ui_input::KeyInput) -> &str {
    let logical = key.logical.0.as_ref();
    let chorded = key.modifiers.control || key.modifiers.meta;
    let single = {
        let mut chars = logical.chars();
        chars.next().is_some() && chars.next().is_none()
    };
    if !chorded || !single || logical.is_ascii() {
        return logical;
    }
    const LETTERS: [&str; 26] = [
        "a", "b", "c", "d", "e", "f", "g", "h", "i", "j", "k", "l", "m", "n", "o", "p", "q", "r",
        "s", "t", "u", "v", "w", "x", "y", "z",
    ];
    const DIGITS: [&str; 10] = ["0", "1", "2", "3", "4", "5", "6", "7", "8", "9"];
    let physical = key.physical.0.as_ref();
    let at = |prefix: &str| {
        physical
            .strip_prefix(prefix)
            .filter(|rest| rest.len() == 1)
            .and_then(|rest| rest.bytes().next())
    };
    if let Some(letter) = at("Key").filter(u8::is_ascii_uppercase) {
        return LETTERS[usize::from(letter - b'A')];
    }
    if let Some(digit) = at("Digit").filter(u8::is_ascii_digit) {
        return DIGITS[usize::from(digit - b'0')];
    }
    logical
}

/// Input routing state of one context.
#[derive(Debug)]
pub(crate) struct InputState {
    sources: HashMap<InputSourceId, SourceState>,
    next_pointer: u64,
    counters: InputCounters,
    /// The button whose press a pointer holds. A mouse is one pointer for
    /// all its buttons; another button going down or up meanwhile must not
    /// end or activate what the first started.
    pub(super) pressed_buttons: HashMap<(DocumentId, u64), i16>,
}

impl Default for InputState {
    fn default() -> Self {
        Self {
            sources: HashMap::new(),
            next_pointer: 1,
            counters: InputCounters::default(),
            pressed_buttons: HashMap::new(),
        }
    }
}

impl InputState {
    fn source(&mut self, source: InputSourceId) -> Option<&mut SourceState> {
        self.sources.get_mut(&source)
    }

    /// The local id for a platform pointer, allocated on first sight so the
    /// first event of a pointer never reads another pointer's state.
    fn resolve_pointer(
        &mut self,
        source: InputSourceId,
        device: DeviceId,
        payload: &InputPayload,
    ) -> Option<u64> {
        let (pointer, sample) = match payload {
            InputPayload::Pointer(pointer) => (pointer.pointer_id, Some(pointer)),
            InputPayload::Wheel(wheel) => (wheel.pointer_id, None),
            InputPayload::PointerEnter { pointer_id, .. }
            | InputPayload::PointerLeave { pointer_id } => (*pointer_id, None),
            _ => return None,
        };
        let next = &mut self.next_pointer;
        let state = self.sources.get_mut(&source)?;
        let identity = state.pointers.entry((device, pointer)).or_insert_with(|| {
            let local = *next;
            *next = next.wrapping_add(1).max(1);
            PointerIdentity {
                local,
                pointer_type: PointerType::Mouse,
                is_primary: true,
            }
        });
        // Enter, leave and wheel carry no tool metadata; only a pointer
        // sample updates it.
        if let Some(sample) = sample {
            identity.pointer_type = sample.pointer_type;
            identity.is_primary = sample.is_primary;
        }
        Some(identity.local)
    }
}

impl AppContext {
    /// Bind `source` to `document` at `generation`. Rebinding at a later
    /// generation cancels what the previous binding left pressed or captured
    /// only if the caller unbound it first ([`Self::unbind_input_source`]);
    /// this call forgets it. The same binding again is a no-op.
    pub fn bind_input_source(
        &mut self,
        source: InputSourceId,
        generation: EndpointGeneration,
        document: DocumentId,
    ) -> Result<(), InputBindError> {
        if let Some(bound) = self.input.sources.get(&source) {
            if generation < bound.generation {
                return Err(InputBindError::StaleGeneration {
                    bound: bound.generation,
                });
            }
            if generation == bound.generation {
                return if document == bound.document {
                    Ok(())
                } else {
                    Err(InputBindError::DocumentRebind {
                        bound: bound.document,
                    })
                };
            }
            // A newer generation is the source starting over: what the old
            // one held (a press, a capture, a preedit) is cancelled as an
            // unbind cancels it, not dropped with its state.
            let now = self.world.animation_now;
            let _ = self.unbind_input_source(source, now);
        }
        self.input
            .sources
            .insert(source, SourceState::new(generation, document));
        Ok(())
    }

    /// Unbind `source`: its presses and captures are cancelled through the
    /// same path an explicit pointer cancel takes, then forgotten, and a file
    /// drag it has hovering ends. An active IME preedit is cancelled while
    /// the old document focus is still available. Document focus is left
    /// alone. Returns the document it was bound to.
    pub fn unbind_input_source(
        &mut self,
        source: InputSourceId,
        now: Duration,
    ) -> Result<Option<DocumentId>, FrameworkError> {
        let Some((document, should_cancel_composition)) =
            self.input.sources.get(&source).map(|state| {
                let other_focused = self.input.sources.iter().any(|(id, other)| {
                    *id != source
                        && other.document == state.document
                        && other.focused
                        && !other.disconnected
                });
                (state.document, !other_focused)
            })
        else {
            return Ok(None);
        };
        self.cancel_source_pointers(source, document, None, now)?;
        self.cancel_source_file_drag(source, document)?;
        if should_cancel_composition {
            // Unbinding can happen without a preceding Focus(false) (window
            // close, host takeover, or a document replacement). Clear the
            // preedit while the old document focus is still available.
            self.dispatch_composition(document, &CompositionInput::End)?;
        }
        self.input.sources.remove(&source);
        Ok(Some(document))
    }

    /// The generation and document `source` is bound to.
    pub fn input_binding(&self, source: InputSourceId) -> Option<(EndpointGeneration, DocumentId)> {
        self.input
            .sources
            .get(&source)
            .map(|state| (state.generation, state.document))
    }

    pub fn input_counters(&self) -> InputCounters {
        self.input.counters
    }

    /// Route one event from a bound source.
    pub fn route_input(
        &mut self,
        event: &CanonicalInputEvent,
        services: &mut dyn HostServices,
        text_shaper: Option<&mut dyn TextShaper>,
    ) -> Result<InputRouteOutcome, InputRouteError> {
        let started = Instant::now();
        let result = self.route_validated(event, services, text_shaper);
        match &result {
            Ok(_) => {
                self.input.counters.events_routed += 1;
                nana_diagnostics::metric!(
                    nana_diagnostics::framework::runtime::INPUT_ROUTED_DISPATCHES
                );
            }
            Err(InputRouteError::Dispatch(_)) => {}
            Err(_) => {
                self.input.counters.events_rejected += 1;
                nana_diagnostics::metric!(
                    nana_diagnostics::framework::runtime::INPUT_ROUTE_REJECTED
                );
            }
        }
        nana_diagnostics::metric!(
            nana_diagnostics::framework::runtime::INPUT_ROUTE_NS,
            started.elapsed()
        );
        result
    }

    /// Route every event `endpoint` holds, in order, into `routed`. Each is
    /// taken off the endpoint before it is routed, so nothing an event does
    /// can leave it stuck at the head. Returns how many were routed.
    pub fn drain_input(
        &mut self,
        endpoint: &mut InputEndpoint,
        services: &mut dyn HostServices,
        mut text_shaper: Option<&mut dyn TextShaper>,
        routed: &mut Vec<RoutedEvent>,
    ) -> usize {
        let mut count = 0;
        while let Some(event) =
            self.route_next(endpoint, services, reborrow_text_shaper(&mut text_shaper))
        {
            routed.push(event);
            count += 1;
        }
        count
    }

    /// Take the next event off `endpoint` and route it. A host whose program
    /// observes input routes one event, lets the program observe it, then
    /// routes the next: what the program does with a key press (stop its
    /// text with [`Self::suppress_text_for`]) lands before that text routes.
    pub fn route_next(
        &mut self,
        endpoint: &mut InputEndpoint,
        services: &mut dyn HostServices,
        text_shaper: Option<&mut dyn TextShaper>,
    ) -> Option<RoutedEvent> {
        let event = endpoint.pop()?;
        let result = self.route_input(&event, services, text_shaper);
        let event = match &result {
            Ok(outcome) => outcome.localize(event),
            Err(_) => event,
        };
        Some(RoutedEvent { event, result })
    }

    /// Drop the text the key press `key` of `source` typed, as a page's
    /// `keydown.preventDefault()` does: a later [`InputPayload::Text`] whose
    /// `key` is `key` inserts nothing. Only the latest press is remembered.
    pub fn suppress_text_for(&mut self, source: InputSourceId, key: InputSequence) {
        if let Some(state) = self.input.sources.get_mut(&source) {
            state.handled_key = Some(key);
        }
    }

    /// Bring `source`'s cursor and text-input state up to date with a world
    /// that changed without an input event: a frame laid out, a caret moved,
    /// a node under a resting pointer went away. Hosts call it after a frame.
    pub fn refresh_input_effects(
        &mut self,
        source: InputSourceId,
        services: &mut dyn HostServices,
    ) {
        let Some(state) = self.input.sources.get(&source) else {
            return;
        };
        let document = state.document;
        let pointer = state.pointer;
        if let Some((local, x, y)) = pointer {
            let target = match self.world.pointer_capture(document, local) {
                Some(captured) => Some(captured),
                None => {
                    // What lies under a resting pointer moves without it: a
                    // wheel scrolled, layout shifted, a row animated in.
                    // Hover follows, as a browser's does after the frame.
                    let under = self.world.hit_test(document, x, y);
                    if under != self.world.pointer_hover(document, local) {
                        let now = self.world.animation_now;
                        let _ = self.set_pointer_hover_at(document, local, under, now);
                    }
                    under
                }
            };
            self.sync_cursor(source, document, x, y, target, None, services);
        }
        self.sync_text_input(source, document, false, services);
    }

    fn route_validated(
        &mut self,
        event: &CanonicalInputEvent,
        services: &mut dyn HostServices,
        text_shaper: Option<&mut dyn TextShaper>,
    ) -> Result<InputRouteOutcome, InputRouteError> {
        let meta = event.metadata;
        let source = meta.source;
        let state = self
            .input
            .source(source)
            .ok_or(InputRouteError::UnknownSource)?;
        if state.generation != meta.generation {
            return Err(InputRouteError::StaleGeneration);
        }
        let source_connecting = matches!(event.payload, InputPayload::SourceConnected);
        if (state.disconnected && !source_connecting)
            || (state.disconnected_devices.contains(&meta.device)
                && !matches!(
                    event.payload,
                    InputPayload::DeviceConnected
                        | InputPayload::SourceConnected
                        | InputPayload::SourceDisconnected
                ))
        {
            return Err(InputRouteError::Disconnected);
        }
        if let Some((sequence, timestamp)) = state.last {
            if meta.sequence <= sequence {
                return Err(InputRouteError::OutOfOrder);
            }
            if meta.timestamp < timestamp {
                return Err(InputRouteError::TimestampRegression);
            }
        }
        state.last = Some((meta.sequence, meta.timestamp));
        let document = state.document;
        let now = meta.timestamp.as_duration();
        let local = self
            .input
            .resolve_pointer(source, meta.device, &event.payload);
        let focus_before = self.world.focused(document);
        let capture_before = local.and_then(|local| self.world.pointer_capture(document, local));
        let hover_before = local.and_then(|local| self.world.pointer_hover(document, local));
        let work_before = self.world.pending_work_revision();
        let hits_before = self.world.hit_test_queries();

        let mut landed = None;
        let disposition = match &event.payload {
            InputPayload::Pointer(pointer) => {
                let pointer = PointerInput {
                    pointer_id: PointerId(local.expect("pointer resolved above")),
                    ..*pointer
                };
                let disposition =
                    self.dispatch_pointer(document, &pointer, now, text_shaper, &mut landed)?;
                self.after_pointer(source, document, &pointer, landed, now, services)?;
                disposition
            }
            // A wheel moves nothing under the cursor it did not already show;
            // the next pointer sample re-derives it.
            InputPayload::Wheel(wheel) => {
                let local = local.expect("pointer resolved above");
                let wheel = WheelInput {
                    pointer_id: PointerId(local),
                    ..*wheel
                };
                self.dispatch_wheel(document, &wheel, &mut landed)?
            }
            InputPayload::PointerEnter { x, y, .. } => {
                let local = local.expect("pointer resolved above");
                let target = self.world.hit_test(document, *x, *y);
                self.set_pointer_location(document, local, Some((*x, *y)));
                self.set_pointer_hover_at(document, local, target, now)?;
                if let Some(state) = self.input.source(source) {
                    state.pointer = Some((local, *x, *y));
                }
                self.sync_cursor(source, document, *x, *y, target, Some(now), services);
                InputDisposition::default()
            }
            InputPayload::PointerLeave { .. } => {
                let local = local.expect("pointer resolved above");
                self.set_pointer_location(document, local, None);
                self.set_pointer_hover_at(document, local, None, now)?;
                // The host restores its own cursor once the pointer is gone;
                // the next enter derives it afresh.
                if let Some(state) = self.input.source(source) {
                    state.pointer = None;
                    state.effects.forget_cursor();
                }
                InputDisposition::default()
            }
            InputPayload::Key(key) => {
                let disposition = self.dispatch_keystroke(
                    document,
                    KeyStroke {
                        pressed: key.state == KeyState::Pressed,
                        key: shortcut_key(key),
                        text: None,
                        repeat: key.repeat,
                        modifiers: key.modifiers,
                        canonical: Some(key),
                    },
                    services,
                    text_shaper,
                )?;
                if key.state == KeyState::Pressed
                    && disposition.handled
                    && let Some(state) = self.input.source(source)
                {
                    state.handled_key = Some(meta.sequence);
                }
                disposition
            }
            InputPayload::Text(committed) => {
                let suppressed = committed.key.is_some()
                    && self
                        .input
                        .source(source)
                        .is_some_and(|state| state.handled_key == committed.key);
                if suppressed {
                    self.input.counters.text_suppressed += 1;
                    InputDisposition {
                        handled: false,
                        prevent_default: true,
                    }
                } else if committed.text.is_empty() {
                    InputDisposition::default()
                } else {
                    self.dispatch_keystroke(
                        document,
                        KeyStroke::text(&committed.text),
                        services,
                        text_shaper,
                    )?
                }
            }
            InputPayload::Composition(composition) => {
                self.dispatch_composition(document, composition)?
            }
            InputPayload::FileDrag(drag) => {
                let (changed, target) =
                    self.file_drag(document, drag.kind, &drag.paths, drag.position)?;
                landed = target;
                if let Some(state) = self.input.source(source) {
                    state.file_drag =
                        drag.kind == nana_ui_core::FileDragKind::Hover && drag.position.is_some();
                }
                InputDisposition {
                    handled: changed,
                    prevent_default: false,
                }
            }
            InputPayload::Focus { focused } => {
                self.route_focus(source, document, *focused, now, services)?;
                InputDisposition::default()
            }
            InputPayload::DeviceDisconnected => {
                self.cancel_source_pointers(source, document, Some(meta.device), now)?;
                if let Some(state) = self.input.source(source) {
                    state.disconnected_devices.insert(meta.device);
                }
                InputDisposition::default()
            }
            InputPayload::DeviceConnected => {
                if let Some(state) = self.input.source(source) {
                    state.disconnected_devices.remove(&meta.device);
                }
                InputDisposition::default()
            }
            InputPayload::SourceDisconnected => {
                self.route_source_disconnect(source, document, now)?;
                InputDisposition::default()
            }
            InputPayload::SourceConnected => {
                if let Some(state) = self.input.source(source) {
                    state.disconnected = false;
                }
                InputDisposition::default()
            }
        };

        if !matches!(event.payload, InputPayload::Pointer(_)) {
            // Focus, text and composition move the caret and the focused
            // editor; a pointer sample already synced in `after_pointer`.
            self.sync_text_input(source, document, false, services);
        }
        let hits = self.world.hit_test_queries() - hits_before;
        if hits > 0 {
            nana_diagnostics::metric!(nana_diagnostics::framework::runtime::INPUT_HIT_TESTS, hits);
        }
        if focus_before != self.world.focused(document) {
            nana_diagnostics::metric!(nana_diagnostics::framework::runtime::INPUT_FOCUS_CHANGES);
        }
        if let Some(local) = local {
            if capture_before != self.world.pointer_capture(document, local) {
                nana_diagnostics::metric!(
                    nana_diagnostics::framework::runtime::INPUT_CAPTURE_CHANGES
                );
            }
            if hover_before != self.world.pointer_hover(document, local) {
                self.input.counters.hover_changes += 1;
                nana_diagnostics::metric!(
                    nana_diagnostics::framework::runtime::INPUT_HOVER_CHANGES
                );
            }
        }
        if self.has_auto_overlays() {
            let activity = match &event.payload {
                InputPayload::Pointer(pointer)
                    if matches!(
                        pointer.phase,
                        PointerPhase::Move | PointerPhase::Down | PointerPhase::Up
                    ) =>
                {
                    OverlayActivity::Pointer {
                        x: pointer.x,
                        y: pointer.y,
                        target: landed,
                    }
                }
                InputPayload::PointerEnter { x, y, .. } => OverlayActivity::Pointer {
                    x: *x,
                    y: *y,
                    target: None,
                },
                InputPayload::PointerLeave { .. } => OverlayActivity::Leave,
                InputPayload::Key(key) if key.state == KeyState::Pressed => OverlayActivity::Key,
                _ => OverlayActivity::None,
            };
            // A key press does not move the clock on its own; the idle
            // deadline counts from this event.
            self.component_lifecycle.now = self.component_lifecycle.now.max(now);
            self.route_auto_overlays(document, activity);
        }
        // Handlers wrote signals; apply their bindings before deciding
        // whether this event invalidated the frame.
        self.flush_reactive()?;
        Ok(InputRouteOutcome {
            handled: disposition.handled,
            prevent_default: disposition.prevent_default,
            pointer_hit: landed,
            invalidated_work: self.world.pending_work_revision() != work_before,
            pointer_id: local,
        })
    }

    /// What a pointer sample leaves behind: the source's last position, the
    /// cursor, and for a touch or pen that lifted, nothing at all.
    fn after_pointer(
        &mut self,
        source: InputSourceId,
        document: DocumentId,
        pointer: &PointerInput,
        landed: Option<StableNodeId>,
        now: Duration,
        services: &mut dyn HostServices,
    ) -> Result<(), FrameworkError> {
        let local = pointer.pointer_id.0;
        let lifted = pointer.pointer_type != PointerType::Mouse
            && matches!(pointer.phase, PointerPhase::Up | PointerPhase::Cancel);
        if lifted {
            // A finger or stylus that lifted hovers nothing, and its next
            // touch is a new pointer: leave no hover or position behind.
            self.set_pointer_location(document, local, None);
            self.set_pointer_hover_at(document, local, None, now)?;
            if let Some(state) = self.input.source(source) {
                state.pointers.retain(|_, identity| identity.local != local);
                if state
                    .pointer
                    .is_some_and(|(pointer, _, _)| pointer == local)
                {
                    state.pointer = None;
                }
            }
        } else {
            if let Some(state) = self.input.source(source) {
                state.pointer = Some((local, pointer.x, pointer.y));
            }
            let throttle = (pointer.phase == PointerPhase::Move).then_some(now);
            self.sync_cursor(
                source, document, pointer.x, pointer.y, landed, throttle, services,
            );
        }
        if pointer.phase != PointerPhase::Move {
            // Presses and releases focus and edit; moves never do.
            self.sync_text_input(source, document, false, services);
        }
        Ok(())
    }

    /// Window focus for `source`. Losing it cancels what the source held
    /// pressed or captured and any active IME preedit when no other source
    /// still has focus: the document keeps its focused control, so returning
    /// to the window types where it left off. Gaining it hands the host the
    /// text-input state again, which the platform dropped with focus.
    fn route_focus(
        &mut self,
        source: InputSourceId,
        document: DocumentId,
        focused: bool,
        now: Duration,
        services: &mut dyn HostServices,
    ) -> Result<(), FrameworkError> {
        if let Some(state) = self.input.source(source) {
            state.focused = focused;
            if focused {
                state.focus_ever = true;
            }
        }
        if focused {
            self.sync_text_input(source, document, true, services);
        } else {
            self.cancel_source_pointers(source, document, None, now)?;
            self.cancel_source_file_drag(source, document)?;
            // Losing the host window cancels an in-progress IME preedit. A
            // later commit must never land after focus moved away; the page
            // observes the corresponding composition end through the Focus
            // event's routed observation.
            let other_focused = self.input.sources.iter().any(|(id, other)| {
                *id != source && other.document == document && other.focused && !other.disconnected
            });
            if !other_focused {
                self.dispatch_composition(document, &CompositionInput::End)?;
            }
        }
        Ok(())
    }

    /// End the file drag `source` has hovering, as its leaving would.
    fn cancel_source_file_drag(
        &mut self,
        source: InputSourceId,
        document: DocumentId,
    ) -> Result<(), FrameworkError> {
        let Some(state) = self.input.source(source) else {
            return Ok(());
        };
        if std::mem::replace(&mut state.file_drag, false) {
            self.file_drag(document, nana_ui_core::FileDragKind::Cancel, &[], None)?;
        }
        Ok(())
    }

    /// The source went away. Its pointers are cancelled; if no other focused
    /// source still drives the document, its focus goes too, since nothing
    /// is left to type into it.
    fn route_source_disconnect(
        &mut self,
        source: InputSourceId,
        document: DocumentId,
        now: Duration,
    ) -> Result<(), FrameworkError> {
        self.cancel_source_pointers(source, document, None, now)?;
        self.cancel_source_file_drag(source, document)?;
        let Some(state) = self.input.source(source) else {
            return Ok(());
        };
        let had_focus = state.focus_ever;
        state.focused = false;
        state.disconnected = true;
        let other_focused = self.input.sources.iter().any(|(id, other)| {
            *id != source && other.document == document && other.focused && !other.disconnected
        });
        if !other_focused {
            // A source may disappear without first sending Focus(false). End
            // the preedit before clear_focus makes the focused editor
            // undiscoverable to the text-input router.
            self.dispatch_composition(document, &CompositionInput::End)?;
        }
        if had_focus && !other_focused {
            self.clear_focus(document)?;
        }
        Ok(())
    }

    /// Cancel every press and capture `source` holds, on `device` or on all
    /// its devices, through the path an explicit pointer cancel takes, then
    /// forget those pointers.
    fn cancel_source_pointers(
        &mut self,
        source: InputSourceId,
        document: DocumentId,
        device: Option<DeviceId>,
        now: Duration,
    ) -> Result<(), FrameworkError> {
        let Some(state) = self.input.sources.get(&source) else {
            return Ok(());
        };
        let identities: Vec<PointerIdentity> = state
            .pointers
            .iter()
            .filter(|((candidate, _), _)| device.is_none_or(|wanted| wanted == *candidate))
            .map(|(_, identity)| *identity)
            .collect();
        for identity in identities {
            let pointer = identity.local;
            let (x, y) = self
                .pointer_position(document, pointer)
                .unwrap_or((0.0, 0.0));
            let held = self.world.pointer_capture(document, pointer).is_some()
                || self.world.pointer_press(document, pointer).is_some();
            if held {
                // Components hear a lifecycle cancel exactly as they hear an
                // explicit one, before their capture is revoked.
                let cancel = PointerInput {
                    pointer_id: PointerId(pointer),
                    pointer_type: identity.pointer_type,
                    is_primary: identity.is_primary,
                    ..PointerInput::mouse(PointerPhase::Cancel, x, y)
                };
                let mut landed = None;
                self.dispatch_pointer(document, &cancel, now, None, &mut landed)?;
            }
            self.release_pointer_capture(document, pointer);
            self.release_pointer(document, pointer);
            // A press outside a blocking overlay holds no press or capture,
            // so no cancel reached the overlay: forget its sequence here.
            self.component_lifecycle
                .overlay_pointer_sequences
                .remove(&(document, pointer));
            self.component_lifecycle
                .overlay_outside_presses
                .remove(&(document, pointer));
            self.set_pointer_location(document, pointer, None);
            self.set_pointer_hover_at(document, pointer, None, now)?;
        }
        if let Some(state) = self.input.sources.get_mut(&source) {
            state
                .pointers
                .retain(|(candidate, _), _| device.is_some_and(|wanted| wanted != *candidate));
            if device.is_none() {
                state.pointer = None;
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests;
