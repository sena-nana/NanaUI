//! Source-local semantic input. Hosts lower native messages once at this boundary.
//!
//! Timestamps are nanoseconds in the Runtime's animation-clock domain: the
//! router reads each event's time from its timestamp, so tooltip delays, hover
//! probes and click timing run on the same clock the host animates with.
//! Sequence is strictly increasing within an endpoint generation, across all
//! its devices; [`InputSequencer`] stamps both. This module owns no thread,
//! timer, OS window, or document state.

use crate::{InputModifiers, PointerPhase, PointerType};
use nana_diagnostics::metric;
use serde::{Deserialize, Serialize};
use std::{
    borrow::Cow,
    collections::{HashMap, HashSet, VecDeque},
    time::Duration,
};

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

impl PointerInput {
    /// A primary mouse sample at `(x, y)`, with the screen position equal to
    /// the client one. A press or release uses the primary button; tests and
    /// headless hosts adjust the rest with struct update syntax.
    pub fn mouse(phase: PointerPhase, x: f32, y: f32) -> Self {
        let pressed = phase == PointerPhase::Down;
        Self {
            phase,
            pointer_id: PointerId(1),
            pointer_type: PointerType::Mouse,
            x,
            y,
            screen_x: x,
            screen_y: y,
            button: if matches!(phase, PointerPhase::Down | PointerPhase::Up) {
                0
            } else {
                -1
            },
            buttons: u16::from(pressed),
            pressure: if pressed { 0.5 } else { 0.0 },
            tangential_pressure: 0.0,
            tilt_x: 0,
            tilt_y: 0,
            twist: 0,
            is_primary: true,
            activation_click: false,
            modifiers: InputModifiers::default(),
        }
    }
}

impl KeyInput {
    /// A key transition whose physical and logical names are static, as named
    /// keys and headless fixtures are.
    pub fn named(
        physical: &'static str,
        logical: &'static str,
        state: KeyState,
        modifiers: InputModifiers,
    ) -> Self {
        Self {
            physical: PhysicalKey(Cow::Borrowed(physical)),
            logical: LogicalKey(Cow::Borrowed(logical)),
            state,
            repeat: false,
            modifiers,
        }
    }

    pub fn is_pressed(&self) -> bool {
        self.state == KeyState::Pressed
    }
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
    Text(CommittedText),
    Composition(CompositionInput),
    Focus {
        focused: bool,
    },
    DeviceConnected,
    DeviceDisconnected,
    SourceConnected,
    SourceDisconnected,
}

/// Text the platform committed, and the key press that produced it when it
/// came with one. A key whose press the Runtime handled (a shortcut, focus
/// traversal, a submitted field) inserts no text: the router drops the text
/// that names it, the way a browser skips `input` after a prevented
/// `keydown`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CommittedText {
    pub text: String,
    /// Sequence of the [`InputPayload::Key`] press this text belongs to.
    pub key: Option<InputSequence>,
}

impl CommittedText {
    /// Text that came with no key press (paste from a soft keyboard, a
    /// synthesized insert).
    pub fn new(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            key: None,
        }
    }
}

impl InputPayload {
    /// Pointer moves and wheel deltas: the endpoint may merge adjacent ones,
    /// so a host can defer draining them to the end of its event-loop turn.
    /// Every other payload is a transition and is drained at once.
    pub fn is_coalescible(&self) -> bool {
        matches!(
            self,
            Self::Pointer(PointerInput {
                phase: PointerPhase::Move,
                ..
            }) | Self::Wheel(_)
        )
    }

    /// Owned variable payload capacity, excluding the inline event itself.
    pub fn allocation_bytes(&self) -> usize {
        match self {
            Self::Text(CommittedText { text, .. })
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

/// Stamps one source's events with its generation, a strictly increasing
/// sequence and a timestamp. Every host keeps one per source instead of
/// counting sequences by hand.
#[derive(Debug, Clone)]
pub struct InputSequencer {
    source: InputSourceId,
    generation: EndpointGeneration,
    last: u64,
}

impl InputSequencer {
    pub fn new(source: InputSourceId, generation: EndpointGeneration) -> Self {
        Self {
            source,
            generation,
            last: 0,
        }
    }

    pub fn source(&self) -> InputSourceId {
        self.source
    }

    pub fn generation(&self) -> EndpointGeneration {
        self.generation
    }

    /// Metadata for the next event. `now` is the Runtime clock time the event
    /// happened at.
    ///
    /// # Panics
    /// When the generation's sequence space is exhausted; call [`Self::advance`]
    /// long before (a source would need 2^64 events).
    pub fn stamp(&mut self, device: DeviceId, now: Duration) -> InputMetadata {
        self.last = self
            .last
            .checked_add(1)
            .expect("input sequence exhausted; advance the endpoint generation");
        InputMetadata {
            source: self.source,
            device,
            generation: self.generation,
            sequence: InputSequence(self.last),
            timestamp: InputTimestamp(now.as_nanos().min(u128::from(u64::MAX)) as u64),
        }
    }

    /// A new endpoint generation: sequences restart and nothing stamped
    /// before is accepted again.
    pub fn advance(&mut self) -> EndpointGeneration {
        self.generation = EndpointGeneration(self.generation.0.saturating_add(1));
        self.last = 0;
        self.generation
    }
}

impl InputTimestamp {
    /// The Runtime clock time this timestamp stands for.
    pub fn as_duration(self) -> Duration {
        Duration::from_nanos(self.0)
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
    fn stale_and_capacity_return_owned_events_and_count_rejections() {
        let mut endpoint = InputEndpoint::new(InputSourceId(1), EndpointGeneration(1), 1, 1);
        let mut text = meta(1);
        text.timestamp = InputTimestamp(1);
        let event = CanonicalInputEvent {
            metadata: text,
            payload: InputPayload::Text(CommittedText::new("hello")),
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
        assert!(!matches!(key, InputPayload::Text(_)));
        assert!(matches!(
            InputPayload::Composition(CompositionInput::Start),
            InputPayload::Composition(_)
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
    fn the_sequencer_orders_events_and_restarts_per_generation() {
        let mut sequencer = InputSequencer::new(InputSourceId(4), EndpointGeneration(1));
        let first = sequencer.stamp(DeviceId(1), Duration::from_millis(16));
        let second = sequencer.stamp(DeviceId(2), Duration::from_millis(16));
        assert_eq!(first.sequence, InputSequence(1));
        assert_eq!(second.sequence, InputSequence(2));
        assert_eq!(first.timestamp.as_duration(), Duration::from_millis(16));
        let mut endpoint = InputEndpoint::new(InputSourceId(4), EndpointGeneration(1), 4, 0);
        endpoint
            .push(CanonicalInputEvent {
                metadata: second,
                payload: InputPayload::Focus { focused: true },
            })
            .unwrap();
        assert_eq!(
            endpoint
                .push(CanonicalInputEvent {
                    metadata: first,
                    payload: InputPayload::Focus { focused: false },
                })
                .unwrap_err()
                .reason,
            InputRejection::OutOfOrder
        );
        assert_eq!(sequencer.advance(), EndpointGeneration(2));
        let restarted = sequencer.stamp(DeviceId(1), Duration::ZERO);
        assert_eq!(restarted.sequence, InputSequence(1));
        assert_eq!(restarted.generation, EndpointGeneration(2));
    }

    #[test]
    fn only_moves_and_wheel_are_coalescible() {
        assert!(
            InputPayload::Pointer(PointerInput::mouse(PointerPhase::Move, 1.0, 2.0))
                .is_coalescible()
        );
        assert!(
            !InputPayload::Pointer(PointerInput::mouse(PointerPhase::Down, 1.0, 2.0))
                .is_coalescible()
        );
        assert!(!InputPayload::Text(CommittedText::new("a")).is_coalescible());
        assert!(!InputPayload::Focus { focused: true }.is_coalescible());
    }
}
