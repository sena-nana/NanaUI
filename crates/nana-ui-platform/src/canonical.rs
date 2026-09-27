//! Source-local semantic input. Hosts lower native messages once at this boundary.
//!
//! Timestamps use a host-selected monotonic epoch (never wall time). Sequence is
//! strictly increasing within an endpoint generation, across all its devices.
//! This module owns no thread, timer, OS window, or document state.

use crate::{InputModifiers, PointerPhase, PointerType};
use nana_diagnostics::metric;
use serde::{Deserialize, Serialize};
use std::{
    borrow::Cow,
    collections::{HashMap, HashSet, VecDeque},
};

/// Version of the public canonical input contract. Serialized or FFI hosts
/// must negotiate this value before exchanging events; Rust enum layout is not
/// itself a wire format.
pub const CANONICAL_INPUT_CONTRACT_VERSION: u32 = 1;

macro_rules! identity {
    ($($name:ident),+ $(,)?) => {$ (
        #[repr(transparent)]
        #[derive(
            Debug, Default, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize,
        )]
        pub struct $name(pub u64);
    )+};
}
identity!(
    InputSourceId,
    DeviceId,
    PointerId,
    EndpointGeneration,
    InputSequence,
    InputTimestamp
);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct InputMetadata {
    pub source: InputSourceId,
    pub device: DeviceId,
    pub generation: EndpointGeneration,
    pub sequence: InputSequence,
    /// Nanoseconds from the source's monotonic epoch. Equal timestamps are valid.
    pub timestamp: InputTimestamp,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CanonicalInputEvent {
    pub metadata: InputMetadata,
    pub payload: InputPayload,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct PointerInput {
    pub phase: PointerPhase,
    pub pointer_id: PointerId,
    pub pointer_type: PointerType,
    pub x: f32,
    pub y: f32,
    pub screen_x: f32,
    pub screen_y: f32,
    pub button: i16,
    pub buttons: u16,
    pub pressure: f32,
    pub tangential_pressure: f32,
    pub tilt_x: i16,
    pub tilt_y: i16,
    pub twist: u16,
    pub is_primary: bool,
    pub activation_click: bool,
    pub modifiers: InputModifiers,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WheelUnit {
    Pixels,
    Lines,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct WheelInput {
    pub pointer_id: PointerId,
    pub x: f32,
    pub y: f32,
    pub delta_x: f32,
    pub delta_y: f32,
    pub unit: WheelUnit,
    pub modifiers: InputModifiers,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum KeyState {
    Pressed,
    Released,
}

/// Hardware position, e.g. `KeyQ`; independent of the active keyboard layout.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PhysicalKey(pub Cow<'static, str>);
/// Layout-resolved key, e.g. `q` or `Enter`. This is never committed text.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LogicalKey(pub Cow<'static, str>);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct KeyInput {
    pub physical: PhysicalKey,
    pub logical: LogicalKey,
    pub state: KeyState,
    pub repeat: bool,
    pub modifiers: InputModifiers,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum CompositionInput {
    Enabled,
    Disabled,
    Start,
    /// UTF-8 byte selection; hosts must preserve character boundaries.
    Update {
        text: String,
        selection: Option<(usize, usize)>,
    },
    Commit(String),
    End,
    DeleteSurrounding {
        before_bytes: usize,
        after_bytes: usize,
    },
}

/// Lower the pre-existing platform event into canonical events at the host
/// boundary. A keyboard event may produce two ordered canonical events: the
/// physical/logical key transition and separately committed text. The iterator
/// stores both events inline, so pointer lowering allocates no heap storage.
///
/// # Panics
/// Panics if committed text requires a second sequence after `u64::MAX`. Hosts
/// must advance endpoint generation before sequence exhaustion.
pub fn lower_input_event(
    event: &crate::InputEvent,
    metadata: InputMetadata,
) -> impl ExactSizeIterator<Item = CanonicalInputEvent> {
    let next = |payload| CanonicalInputEvent { metadata, payload };
    let events =
        match event {
            crate::InputEvent::Pointer {
                phase,
                pointer_id,
                pointer_type,
                x,
                y,
                screen_x,
                screen_y,
                button,
                buttons,
                pressure,
                tangential_pressure,
                tilt_x,
                tilt_y,
                twist,
                is_primary,
                activation_click,
                modifiers,
            } => [
                Some(next(InputPayload::Pointer(PointerInput {
                    phase: *phase,
                    pointer_id: PointerId(*pointer_id),
                    pointer_type: *pointer_type,
                    x: *x,
                    y: *y,
                    screen_x: *screen_x,
                    screen_y: *screen_y,
                    button: *button,
                    buttons: *buttons,
                    pressure: *pressure,
                    tangential_pressure: *tangential_pressure,
                    tilt_x: *tilt_x,
                    tilt_y: *tilt_y,
                    twist: *twist,
                    is_primary: *is_primary,
                    activation_click: *activation_click,
                    modifiers: *modifiers,
                }))),
                None,
            ],
            crate::InputEvent::Wheel {
                x,
                y,
                delta_x,
                delta_y,
                line_delta,
                modifiers,
            } => [
                Some(next(InputPayload::Wheel(WheelInput {
                    pointer_id: PointerId(0),
                    x: *x,
                    y: *y,
                    delta_x: *delta_x,
                    delta_y: *delta_y,
                    unit: if *line_delta {
                        WheelUnit::Lines
                    } else {
                        WheelUnit::Pixels
                    },
                    modifiers: *modifiers,
                }))),
                None,
            ],
            crate::InputEvent::Keyboard {
                pressed,
                key,
                text,
                code,
                repeat,
                modifiers,
            } => {
                let key = next(InputPayload::Key(KeyInput {
                    physical: PhysicalKey(Cow::Owned(code.clone())),
                    logical: LogicalKey(Cow::Owned(key.clone())),
                    state: if *pressed {
                        KeyState::Pressed
                    } else {
                        KeyState::Released
                    },
                    repeat: *repeat,
                    modifiers: *modifiers,
                }));
                let text =
                    text.as_deref()
                        .filter(|text| *pressed && !text.is_empty())
                        .map(|text| CanonicalInputEvent {
                            metadata: InputMetadata {
                                sequence: InputSequence(metadata.sequence.0.checked_add(1).expect(
                                    "input sequence exhausted; advance endpoint generation",
                                )),
                                ..metadata
                            },
                            payload: InputPayload::TextInput(text.to_owned()),
                        });
                [Some(key), text]
            }
        };
    LoweredInputEvents(events)
}

struct LoweredInputEvents([Option<CanonicalInputEvent>; 2]);

impl Iterator for LoweredInputEvents {
    type Item = CanonicalInputEvent;

    fn next(&mut self) -> Option<Self::Item> {
        self.0[0].take().or_else(|| self.0[1].take())
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        let remaining = usize::from(self.0[0].is_some()) + usize::from(self.0[1].is_some());
        (remaining, Some(remaining))
    }
}

impl ExactSizeIterator for LoweredInputEvents {}

/// Lower IME lifecycle data without turning it into keyboard input.
pub fn lower_ime_event(
    event: &crate::ImeEvent,
    metadata: InputMetadata,
) -> Option<CanonicalInputEvent> {
    let payload = match event {
        crate::ImeEvent::Enabled => CompositionInput::Enabled,
        crate::ImeEvent::Disabled => CompositionInput::Disabled,
        crate::ImeEvent::Cancelled => CompositionInput::End,
        crate::ImeEvent::Preedit { text, selection } => CompositionInput::Update {
            text: text.clone(),
            selection: *selection,
        },
        crate::ImeEvent::Commit(text) => CompositionInput::Commit(text.clone()),
        crate::ImeEvent::DeleteSurrounding {
            before_bytes,
            after_bytes,
        } => CompositionInput::DeleteSurrounding {
            before_bytes: *before_bytes,
            after_bytes: *after_bytes,
        },
    };
    Some(CanonicalInputEvent {
        metadata,
        payload: InputPayload::Composition(payload),
    })
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum InputPayload {
    /// Touch and pen share the pointer contract, including pressure and tilt.
    Pointer(PointerInput),
    PointerEnter {
        pointer_id: PointerId,
        x: f32,
        y: f32,
    },
    PointerLeave {
        pointer_id: PointerId,
    },
    Wheel(WheelInput),
    Key(KeyInput),
    /// The platform's committed text. Never inferred from `KeyInput`.
    TextInput(String),
    Composition(CompositionInput),
    Focus {
        focused: bool,
    },
    DeviceConnected,
    DeviceDisconnected,
    SourceConnected,
    SourceDisconnected,
}

/// Versioned wire envelope for adapters that cross a process or FFI boundary.
/// Rust enum layout is deliberately not part of the protocol.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CanonicalInputWireEvent {
    pub version: u32,
    pub event: CanonicalInputEvent,
}

impl CanonicalInputWireEvent {
    pub fn new(event: CanonicalInputEvent) -> Self {
        Self {
            version: CANONICAL_INPUT_CONTRACT_VERSION,
            event,
        }
    }

    pub fn into_event(self) -> Result<CanonicalInputEvent, WireInputError> {
        if self.version != CANONICAL_INPUT_CONTRACT_VERSION {
            return Err(WireInputError::UnsupportedVersion(self.version));
        }
        Ok(self.event)
    }

    pub fn to_json(&self) -> Result<String, serde_json::Error> {
        serde_json::to_string(self)
    }

    pub fn from_json(json: &str) -> Result<CanonicalInputEvent, WireInputError> {
        let envelope: Self = serde_json::from_str(json).map_err(WireInputError::Decode)?;
        envelope.into_event()
    }
}

#[derive(Debug)]
pub enum WireInputError {
    UnsupportedVersion(u32),
    Decode(serde_json::Error),
}

impl std::fmt::Display for WireInputError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::UnsupportedVersion(version) => {
                write!(f, "unsupported input contract version {version}")
            }
            Self::Decode(error) => write!(f, "invalid canonical input wire event: {error}"),
        }
    }
}

impl std::error::Error for WireInputError {}

impl InputPayload {
    /// Owned variable payload capacity, excluding the inline event itself.
    pub fn allocation_bytes(&self) -> usize {
        match self {
            Self::TextInput(text)
            | Self::Composition(CompositionInput::Commit(text))
            | Self::Composition(CompositionInput::Update { text, .. }) => text.capacity(),
            Self::Key(key) => {
                #[expect(
                    clippy::ptr_arg,
                    reason = "Cow capacity is part of the endpoint memory budget"
                )]
                fn capacity(s: &Cow<'static, str>) -> usize {
                    match s {
                        Cow::Owned(s) => s.capacity(),
                        Cow::Borrowed(_) => 0,
                    }
                }
                capacity(&key.physical.0).saturating_add(capacity(&key.logical.0))
            }
            _ => 0,
        }
    }
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct InputEndpointCounters {
    pub input_events_received: u64,
    pub input_events_coalesced: u64,
    pub input_events_dropped_stale: u64,
    pub input_events_rejected_capacity: u64,
    /// Cumulative capacity of accepted owned payloads; not allocator telemetry.
    pub input_payload_alloc_bytes: u64,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct InputDeviceCounters {
    pub input_events_received: u64,
    pub input_events_coalesced: u64,
    pub input_events_dropped_stale: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InputRejection {
    WrongSource,
    StaleGeneration,
    OutOfOrder,
    TimestampRegression,
    Disconnected,
    Capacity,
}
#[derive(Debug, Clone, PartialEq)]
pub struct RejectedInput {
    pub reason: InputRejection,
    /// Return ownership so a host can retry a semantic transition after draining.
    pub event: CanonicalInputEvent,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InputEnqueueOutcome {
    Queued,
    Coalesced,
}

/// Bounded, event-driven single-source inbox. External hosts own any cross-thread
/// synchronization/wakeup and authentication. Queue-full returns ownership rather
/// than silently losing key/button/focus/IME transitions. No background polling.
/// At most `max_events` distinct devices can be tracked per generation, including
/// disconnected devices (their tombstones prevent replay). Reset the generation
/// to reclaim device slots; invalid source/generation events never create slots.
#[derive(Debug)]
pub struct InputEndpoint {
    source: InputSourceId,
    generation: EndpointGeneration,
    queue: VecDeque<CanonicalInputEvent>,
    max_events: usize,
    max_payload_bytes: usize,
    payload_bytes: usize,
    last: Option<(InputSequence, InputTimestamp)>,
    disconnected: bool,
    counters: InputEndpointCounters,
    disconnected_devices: HashSet<DeviceId>,
    device_counters: HashMap<DeviceId, InputDeviceCounters>,
}

impl InputEndpoint {
    pub fn new(
        source: InputSourceId,
        generation: EndpointGeneration,
        max_events: usize,
        max_payload_bytes: usize,
    ) -> Self {
        Self {
            source,
            generation,
            queue: VecDeque::with_capacity(max_events),
            max_events,
            max_payload_bytes,
            payload_bytes: 0,
            last: None,
            disconnected: false,
            counters: InputEndpointCounters::default(),
            disconnected_devices: HashSet::new(),
            device_counters: HashMap::new(),
        }
    }
    pub fn source(&self) -> InputSourceId {
        self.source
    }
    pub fn generation(&self) -> EndpointGeneration {
        self.generation
    }
    pub fn counters(&self) -> InputEndpointCounters {
        self.counters
    }
    pub fn device_counters(&self, device: DeviceId) -> InputDeviceCounters {
        self.device_counters
            .get(&device)
            .copied()
            .unwrap_or_default()
    }
    pub fn len(&self) -> usize {
        self.queue.len()
    }
    pub fn is_empty(&self) -> bool {
        self.queue.is_empty()
    }
    pub fn queued_payload_bytes(&self) -> usize {
        self.payload_bytes
    }

    /// Rebind after disconnect/document replacement. Old queued input is discarded;
    /// the caller must cancel old routing state before using the new generation.
    /// Generations never go backwards or repeat, including after u64 exhaustion.
    pub fn reset(&mut self, generation: EndpointGeneration) -> bool {
        if generation <= self.generation {
            return false;
        }
        self.counters.input_events_dropped_stale += self.queue.len() as u64;
        self.queue.clear();
        self.payload_bytes = 0;
        self.last = None;
        self.disconnected = false;
        self.disconnected_devices.clear();
        self.device_counters.clear();
        self.generation = generation;
        true
    }

    #[allow(clippy::result_large_err)]
    pub fn push(
        &mut self,
        event: CanonicalInputEvent,
    ) -> Result<InputEnqueueOutcome, RejectedInput> {
        self.counters.input_events_received += 1;
        metric!(nana_diagnostics::framework::runtime::INPUT_EVENTS);
        let meta = event.metadata;
        if let Some(stats) = self.device_counters.get_mut(&meta.device) {
            stats.input_events_received += 1;
        }
        let reason = if meta.source != self.source {
            Some(InputRejection::WrongSource)
        } else if meta.generation != self.generation {
            Some(InputRejection::StaleGeneration)
        } else if (self.disconnected && !matches!(event.payload, InputPayload::SourceConnected))
            || (self.disconnected_devices.contains(&meta.device)
                && !matches!(
                    event.payload,
                    InputPayload::DeviceConnected
                        | InputPayload::SourceConnected
                        | InputPayload::SourceDisconnected
                ))
        {
            Some(InputRejection::Disconnected)
        } else if self
            .last
            .is_some_and(|(sequence, _)| meta.sequence <= sequence)
        {
            Some(InputRejection::OutOfOrder)
        } else if self
            .last
            .is_some_and(|(_, timestamp)| meta.timestamp < timestamp)
        {
            Some(InputRejection::TimestampRegression)
        } else {
            None
        };
        if let Some(reason) = reason {
            self.counters.input_events_dropped_stale += 1;
            metric!(nana_diagnostics::framework::runtime::INPUT_STALE_DROPPED);
            if let Some(stats) = self.device_counters.get_mut(&meta.device) {
                stats.input_events_dropped_stale += 1;
            }
            return Err(RejectedInput { reason, event });
        }
        if self
            .queue
            .back_mut()
            .is_some_and(|last| coalesce(last, &event))
        {
            self.last = Some((meta.sequence, meta.timestamp));
            self.counters.input_events_coalesced += 1;
            metric!(nana_diagnostics::framework::runtime::INPUT_COALESCED);
            self.device_counters
                .get_mut(&meta.device)
                .expect("queued device is tracked")
                .input_events_coalesced += 1;
            return Ok(InputEnqueueOutcome::Coalesced);
        }
        let bytes = event.payload.allocation_bytes();
        metric!(
            nana_diagnostics::framework::runtime::INPUT_PAYLOAD_BYTES,
            bytes as u64
        );
        if (!self.device_counters.contains_key(&meta.device)
            && self.device_counters.len() >= self.max_events)
            || self.queue.len() >= self.max_events
            || bytes > self.max_payload_bytes.saturating_sub(self.payload_bytes)
        {
            self.counters.input_events_rejected_capacity += 1;
            return Err(RejectedInput {
                reason: InputRejection::Capacity,
                event,
            });
        }
        self.device_counters
            .entry(meta.device)
            .or_insert(InputDeviceCounters {
                input_events_received: 1,
                ..InputDeviceCounters::default()
            });
        match event.payload {
            InputPayload::SourceConnected => self.disconnected = false,
            InputPayload::SourceDisconnected => self.disconnected = true,
            InputPayload::DeviceDisconnected => {
                self.disconnected_devices.insert(meta.device);
            }
            InputPayload::DeviceConnected => {
                self.disconnected_devices.remove(&meta.device);
            }
            _ => {}
        }
        self.last = Some((meta.sequence, meta.timestamp));
        self.payload_bytes += bytes;
        self.counters.input_payload_alloc_bytes = self
            .counters
            .input_payload_alloc_bytes
            .saturating_add(bytes as u64);
        self.queue.push_back(event);
        Ok(InputEnqueueOutcome::Queued)
    }

    /// Inspect the next event without consuming it or releasing its payload budget.
    pub fn front(&self) -> Option<&CanonicalInputEvent> {
        self.queue.front()
    }

    pub fn pop(&mut self) -> Option<CanonicalInputEvent> {
        let event = self.queue.pop_front()?;
        self.payload_bytes -= event.payload.allocation_bytes();
        Some(event)
    }
}

fn coalesce(previous: &mut CanonicalInputEvent, next: &CanonicalInputEvent) -> bool {
    if previous.metadata.source != next.metadata.source
        || previous.metadata.device != next.metadata.device
        || previous.metadata.generation != next.metadata.generation
    {
        return false;
    }
    match (&mut previous.payload, &next.payload) {
        (InputPayload::Pointer(a), InputPayload::Pointer(b))
            if a.phase == PointerPhase::Move
                && b.phase == PointerPhase::Move
                && a.pointer_id == b.pointer_id
                && a.pointer_type == b.pointer_type
                && a.buttons == b.buttons
                && a.button == b.button
                && a.modifiers == b.modifiers
                && a.is_primary == b.is_primary
                && a.activation_click == b.activation_click =>
        {
            *a = *b;
        }
        (InputPayload::Wheel(a), InputPayload::Wheel(b))
            if a.pointer_id == b.pointer_id
                && a.x == b.x
                && a.y == b.y
                && a.unit == b.unit
                && a.modifiers == b.modifiers
                && (a.delta_x + b.delta_x).is_finite()
                && (a.delta_y + b.delta_y).is_finite() =>
        {
            a.delta_x += b.delta_x;
            a.delta_y += b.delta_y;
            a.x = b.x;
            a.y = b.y;
        }
        _ => return false,
    }
    previous.metadata = next.metadata;
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    fn meta(sequence: u64) -> InputMetadata {
        InputMetadata {
            source: InputSourceId(1),
            device: DeviceId(2),
            generation: EndpointGeneration(1),
            sequence: InputSequence(sequence),
            timestamp: InputTimestamp(sequence * 10),
        }
    }
    fn pointer(sequence: u64, phase: PointerPhase) -> CanonicalInputEvent {
        CanonicalInputEvent {
            metadata: meta(sequence),
            payload: InputPayload::Pointer(PointerInput {
                phase,
                pointer_id: PointerId(3),
                pointer_type: PointerType::Mouse,
                x: sequence as f32,
                y: 2.0,
                screen_x: 0.0,
                screen_y: 0.0,
                button: 0,
                buttons: 0,
                pressure: 0.0,
                tangential_pressure: 0.0,
                tilt_x: 0,
                tilt_y: 0,
                twist: 0,
                is_primary: true,
                activation_click: false,
                modifiers: InputModifiers::default(),
            }),
        }
    }

    #[test]
    fn adjacent_moves_coalesce_but_transitions_are_preserved() {
        let mut endpoint = InputEndpoint::new(InputSourceId(1), EndpointGeneration(1), 8, 0);
        assert_eq!(
            endpoint.push(pointer(1, PointerPhase::Move)).unwrap(),
            InputEnqueueOutcome::Queued
        );
        assert_eq!(
            endpoint.push(pointer(2, PointerPhase::Move)).unwrap(),
            InputEnqueueOutcome::Coalesced
        );
        assert_eq!(
            endpoint.push(pointer(3, PointerPhase::Down)).unwrap(),
            InputEnqueueOutcome::Queued
        );
        assert_eq!(endpoint.len(), 2);
        let first = endpoint.pop().unwrap();
        assert!(matches!(
            first.payload,
            InputPayload::Pointer(PointerInput { x: 2.0, .. })
        ));
    }

    #[test]
    fn high_frequency_moves_keep_one_queued_sample_and_no_heap_payload() {
        let mut endpoint = InputEndpoint::new(InputSourceId(1), EndpointGeneration(1), 8, 0);
        for sequence in 1..=1_000 {
            assert!(matches!(
                endpoint.push(pointer(sequence, PointerPhase::Move)),
                Ok(InputEnqueueOutcome::Queued) | Ok(InputEnqueueOutcome::Coalesced)
            ));
        }
        assert_eq!(endpoint.len(), 1);
        assert_eq!(endpoint.queued_payload_bytes(), 0);
        assert_eq!(endpoint.counters().input_events_coalesced, 999);
    }

    #[test]
    fn high_frequency_host_lowering_uses_inline_events() {
        let event = crate::InputEvent::Pointer {
            phase: PointerPhase::Move,
            pointer_id: 3,
            pointer_type: PointerType::Mouse,
            x: 1.0,
            y: 2.0,
            screen_x: 1.0,
            screen_y: 2.0,
            button: 0,
            buttons: 0,
            pressure: 0.0,
            tangential_pressure: 0.0,
            tilt_x: 0,
            tilt_y: 0,
            twist: 0,
            is_primary: true,
            activation_click: false,
            modifiers: InputModifiers::default(),
        };
        let mut endpoint = InputEndpoint::new(InputSourceId(1), EndpointGeneration(1), 8, 0);
        for sequence in 1..=1_000 {
            let mut lowered = lower_input_event(&event, meta(sequence));
            assert_eq!(lowered.len(), 1);
            let canonical = lowered.next().unwrap();
            assert_eq!(canonical.payload.allocation_bytes(), 0);
            assert_eq!(lowered.len(), 0);
            endpoint.push(canonical).unwrap();
        }
        assert_eq!(endpoint.len(), 1);
        assert_eq!(endpoint.counters().input_events_coalesced, 999);
        assert_eq!(endpoint.counters().input_payload_alloc_bytes, 0);
    }

    #[test]
    fn stale_and_capacity_return_owned_events_and_count_rejections() {
        let mut endpoint = InputEndpoint::new(InputSourceId(1), EndpointGeneration(1), 1, 1);
        let mut text = meta(1);
        text.timestamp = InputTimestamp(1);
        let event = CanonicalInputEvent {
            metadata: text,
            payload: InputPayload::TextInput("hello".to_owned()),
        };
        let rejected = endpoint.push(event).unwrap_err();
        assert_eq!(rejected.reason, InputRejection::Capacity);
        assert_eq!(endpoint.counters().input_events_rejected_capacity, 1);
        let stale = CanonicalInputEvent {
            metadata: InputMetadata {
                generation: EndpointGeneration(0),
                sequence: InputSequence(2),
                ..meta(2)
            },
            payload: InputPayload::Focus { focused: true },
        };
        assert_eq!(
            endpoint.push(stale).unwrap_err().reason,
            InputRejection::StaleGeneration
        );
        assert_eq!(endpoint.counters().input_events_dropped_stale, 1);
    }

    #[test]
    fn key_text_and_composition_are_distinct_payloads() {
        let key = InputPayload::Key(KeyInput {
            physical: PhysicalKey(Cow::Borrowed("KeyA")),
            logical: LogicalKey(Cow::Borrowed("a")),
            state: KeyState::Pressed,
            repeat: false,
            modifiers: InputModifiers::default(),
        });
        assert_eq!(key.allocation_bytes(), 0);
        assert!(!matches!(key, InputPayload::TextInput(_)));
        assert!(matches!(
            InputPayload::Composition(CompositionInput::Start),
            InputPayload::Composition(_)
        ));
    }

    #[test]
    fn versioned_wire_round_trip_and_rejects_unknown_version() {
        let event = pointer(7, PointerPhase::Move);
        let wire = CanonicalInputWireEvent::new(event.clone());
        let json = wire.to_json().unwrap();
        assert_eq!(CanonicalInputWireEvent::from_json(&json).unwrap(), event);

        let mut value: serde_json::Value = serde_json::from_str(&json).unwrap();
        value["version"] = serde_json::json!(99);
        assert!(matches!(
            CanonicalInputWireEvent::from_json(&value.to_string()),
            Err(WireInputError::UnsupportedVersion(99))
        ));
    }

    #[test]
    fn generation_reset_discards_old_queue_and_requires_monotonic_generation() {
        let mut endpoint = InputEndpoint::new(InputSourceId(1), EndpointGeneration(1), 2, 0);
        endpoint.push(pointer(1, PointerPhase::Down)).unwrap();
        assert!(endpoint.reset(EndpointGeneration(2)));
        assert!(endpoint.is_empty());
        assert!(!endpoint.reset(EndpointGeneration(1)));
    }

    #[test]
    fn device_disconnect_rejects_later_events_until_reconnected() {
        let mut endpoint = InputEndpoint::new(InputSourceId(1), EndpointGeneration(1), 8, 0);
        let disconnected = CanonicalInputEvent {
            metadata: meta(1),
            payload: InputPayload::DeviceDisconnected,
        };
        endpoint.push(disconnected).unwrap();
        let rejected = endpoint.push(pointer(2, PointerPhase::Move)).unwrap_err();
        assert_eq!(rejected.reason, InputRejection::Disconnected);
        let connected = CanonicalInputEvent {
            metadata: meta(3),
            payload: InputPayload::DeviceConnected,
        };
        endpoint.push(connected).unwrap();
        assert!(endpoint.push(pointer(4, PointerPhase::Move)).is_ok());
        assert_eq!(
            endpoint.device_counters(DeviceId(2)).input_events_received,
            4
        );
        assert_eq!(
            endpoint
                .device_counters(DeviceId(2))
                .input_events_dropped_stale,
            1
        );
    }

    #[test]
    fn source_disconnect_accepts_explicit_reconnect() {
        let mut endpoint = InputEndpoint::new(InputSourceId(1), EndpointGeneration(1), 4, 0);
        let mut disconnected = meta(1);
        disconnected.device = DeviceId(8);
        endpoint
            .push(CanonicalInputEvent {
                metadata: disconnected,
                payload: InputPayload::SourceDisconnected,
            })
            .unwrap();
        let mut connected = meta(2);
        connected.device = DeviceId(8);
        endpoint
            .push(CanonicalInputEvent {
                metadata: connected,
                payload: InputPayload::SourceConnected,
            })
            .unwrap();
        assert!(!endpoint.is_empty());
    }

    #[test]
    fn source_lifecycle_bypasses_device_tombstone_without_reviving_device() {
        let mut endpoint = InputEndpoint::new(InputSourceId(1), EndpointGeneration(1), 8, 0);
        for (sequence, payload) in [
            (1, InputPayload::DeviceDisconnected),
            (2, InputPayload::SourceDisconnected),
            (3, InputPayload::SourceConnected),
        ] {
            endpoint
                .push(CanonicalInputEvent {
                    metadata: meta(sequence),
                    payload,
                })
                .unwrap();
        }
        assert_eq!(
            endpoint
                .push(CanonicalInputEvent {
                    metadata: meta(4),
                    payload: InputPayload::Focus { focused: true },
                })
                .unwrap_err()
                .reason,
            InputRejection::Disconnected
        );
        endpoint
            .push(CanonicalInputEvent {
                metadata: meta(4),
                payload: InputPayload::DeviceConnected,
            })
            .unwrap();
    }

    #[test]
    fn legacy_keyboard_lowers_key_before_committed_text() {
        let event = crate::InputEvent::Keyboard {
            pressed: true,
            key: "a".into(),
            text: Some("あ".into()),
            code: "KeyA".into(),
            repeat: false,
            modifiers: InputModifiers::default(),
        };
        let events: Vec<_> = lower_input_event(&event, meta(7)).collect();
        assert!(matches!(events[0].payload, InputPayload::Key(_)));
        assert!(matches!(events[1].payload, InputPayload::TextInput(_)));
        assert_eq!(events[1].metadata.sequence, InputSequence(8));
    }

    #[test]
    fn released_key_never_commits_text() {
        let event = crate::InputEvent::Keyboard {
            pressed: false,
            key: "a".into(),
            text: Some("a".into()),
            code: "KeyA".into(),
            repeat: false,
            modifiers: InputModifiers::default(),
        };
        let mut events = lower_input_event(&event, meta(1));
        assert_eq!(events.len(), 1);
        assert!(matches!(
            events.next().unwrap().payload,
            InputPayload::Key(KeyInput {
                state: KeyState::Released,
                ..
            })
        ));
        assert_eq!(events.len(), 0);
    }

    #[test]
    fn device_bookkeeping_is_bounded_even_when_drained_or_rejected() {
        let mut endpoint = InputEndpoint::new(InputSourceId(1), EndpointGeneration(1), 2, 0);
        for device in 0..100 {
            let mut event = pointer(device + 1, PointerPhase::Move);
            event.metadata.source = InputSourceId(9);
            event.metadata.device = DeviceId(device);
            assert_eq!(
                endpoint.push(event).unwrap_err().reason,
                InputRejection::WrongSource
            );
        }
        assert!(endpoint.device_counters.is_empty());
        for device in 0..2 {
            let mut event = pointer(device + 1, PointerPhase::Move);
            event.metadata.device = DeviceId(device);
            endpoint.push(event).unwrap();
            endpoint.pop();
        }
        assert_eq!(
            endpoint
                .push(pointer(3, PointerPhase::Move))
                .unwrap_err()
                .reason,
            InputRejection::Capacity
        );
        assert_eq!(endpoint.device_counters.len(), 2);
        assert!(endpoint.reset(EndpointGeneration(2)));
        assert!(endpoint.device_counters.is_empty());
    }

    #[test]
    fn ordering_is_source_local_across_devices_and_capacity_retry_is_allowed() {
        let mut endpoint = InputEndpoint::new(InputSourceId(1), EndpointGeneration(1), 2, 0);
        endpoint.push(pointer(2, PointerPhase::Down)).unwrap();
        let mut other = pointer(1, PointerPhase::Down);
        other.metadata.device = DeviceId(99);
        assert_eq!(
            endpoint.push(other).unwrap_err().reason,
            InputRejection::OutOfOrder
        );
        endpoint.push(pointer(3, PointerPhase::Up)).unwrap();
        let rejected = endpoint.push(pointer(4, PointerPhase::Down)).unwrap_err();
        assert_eq!(rejected.reason, InputRejection::Capacity);
        endpoint.pop();
        endpoint.push(rejected.event).unwrap();
    }

    #[test]
    fn wheel_coalescing_preserves_target_position() {
        let mut endpoint = InputEndpoint::new(InputSourceId(1), EndpointGeneration(1), 4, 0);
        let wheel = |sequence, x| CanonicalInputEvent {
            metadata: meta(sequence),
            payload: InputPayload::Wheel(WheelInput {
                pointer_id: PointerId(0),
                x,
                y: 0.0,
                delta_x: 0.0,
                delta_y: 1.0,
                unit: WheelUnit::Pixels,
                modifiers: InputModifiers::default(),
            }),
        };
        endpoint.push(wheel(1, 1.0)).unwrap();
        assert_eq!(
            endpoint.push(wheel(2, 1.0)).unwrap(),
            InputEnqueueOutcome::Coalesced
        );
        assert_eq!(
            endpoint.push(wheel(3, 2.0)).unwrap(),
            InputEnqueueOutcome::Queued
        );
        assert_eq!(endpoint.len(), 2);
        assert!(matches!(
            endpoint.pop().unwrap().payload,
            InputPayload::Wheel(WheelInput {
                x: 1.0,
                delta_y: 2.0,
                ..
            })
        ));
    }

    #[test]
    fn ime_lowering_does_not_synthesize_a_key_event() {
        let event = crate::ImeEvent::Preedit {
            text: "かな".into(),
            selection: Some((0, 6)),
        };
        let lowered = lower_ime_event(&event, meta(1)).unwrap();
        assert!(matches!(
            lowered.payload,
            InputPayload::Composition(CompositionInput::Update { .. })
        ));
    }

    #[test]
    fn ime_cancel_lowering_preserves_end_without_commit() {
        let lowered = lower_ime_event(&crate::ImeEvent::Cancelled, meta(1)).unwrap();
        assert!(matches!(
            lowered.payload,
            InputPayload::Composition(CompositionInput::End)
        ));
    }
}
