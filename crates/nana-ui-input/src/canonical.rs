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
use nana_ui_core::FileDragKind;
use std::{borrow::Cow, collections::VecDeque, path::PathBuf, time::Duration};

macro_rules! identity {
    ($($name:ident),+ $(,)?) => {$ (
        #[repr(transparent)]
        #[derive(
            Debug, Default, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash,
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InputMetadata {
    pub source: InputSourceId,
    pub device: DeviceId,
    pub generation: EndpointGeneration,
    pub sequence: InputSequence,
    /// Nanoseconds from the source's monotonic epoch. Equal timestamps are valid.
    pub timestamp: InputTimestamp,
}

#[derive(Debug, Clone, PartialEq)]
pub struct CanonicalInputEvent {
    pub metadata: InputMetadata,
    pub payload: InputPayload,
}

impl CanonicalInputEvent {
    /// The pointer a pointer, wheel, enter or leave event is about.
    pub fn pointer_id(&self) -> Option<PointerId> {
        match &self.payload {
            InputPayload::Pointer(pointer) => Some(pointer.pointer_id),
            InputPayload::Wheel(wheel) => Some(wheel.pointer_id),
            InputPayload::PointerEnter { pointer_id, .. }
            | InputPayload::PointerLeave { pointer_id } => Some(*pointer_id),
            _ => None,
        }
    }

    /// The same event about pointer `id`: a router hands observers the id
    /// it keys capture and hover by, not the platform's.
    pub fn with_pointer_id(mut self, id: PointerId) -> Self {
        match &mut self.payload {
            InputPayload::Pointer(pointer) => pointer.pointer_id = id,
            InputPayload::Wheel(wheel) => wheel.pointer_id = id,
            InputPayload::PointerEnter { pointer_id, .. }
            | InputPayload::PointerLeave { pointer_id } => *pointer_id = id,
            _ => {}
        }
        self
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WheelUnit {
    Pixels,
    Lines,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct WheelInput {
    pub pointer_id: PointerId,
    pub x: f32,
    pub y: f32,
    pub delta_x: f32,
    pub delta_y: f32,
    pub unit: WheelUnit,
    pub modifiers: InputModifiers,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyState {
    Pressed,
    Released,
}

/// Hardware position, e.g. `KeyQ`; independent of the active keyboard layout.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PhysicalKey(pub Cow<'static, str>);
/// Layout-resolved key, e.g. `q` or `Enter`. This is never committed text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LogicalKey(pub Cow<'static, str>);

#[derive(Debug, Clone, PartialEq, Eq)]
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

#[derive(Debug, Clone, PartialEq, Eq)]
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

#[derive(Debug, Clone, PartialEq)]
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
    /// Files dragged over the window from outside it.
    FileDrag(FileDragInput),
    Focus {
        focused: bool,
    },
    DeviceConnected,
    DeviceDisconnected,
    SourceConnected,
    SourceDisconnected,
}

/// A platform file drag over the window: files hovering at a point, dropped
/// there, or the drag leaving without a drop.
#[derive(Debug, Clone, PartialEq)]
pub struct FileDragInput {
    pub kind: FileDragKind,
    /// The dragged files. Empty for `Cancel`, and while a platform that
    /// delivers paths late has not sent them yet.
    pub paths: Vec<PathBuf>,
    /// Window position in logical pixels; `None` when the platform reported
    /// none, which ends any hover as a cancel would.
    pub position: Option<(f32, f32)>,
    /// Modifier keys held at this moment. The drag source keeps keyboard
    /// focus, so the host samples the system state where it can.
    pub modifiers: InputModifiers,
}

impl FileDragInput {
    /// The drag left the window, or the platform abandoned it.
    pub fn cancel() -> Self {
        Self {
            kind: FileDragKind::Cancel,
            paths: Vec::new(),
            position: None,
            modifiers: InputModifiers::default(),
        }
    }
}

/// Text the platform committed, and the key press that produced it when it
/// came with one. A key whose press the Runtime handled (a shortcut, focus
/// traversal, a submitted field) inserts no text: the router drops the text
/// that names it, the way a browser skips `input` after a prevented
/// `keydown`.
#[derive(Debug, Clone, PartialEq, Eq)]
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
            Self::FileDrag(drag) => drag.paths.iter().fold(
                drag.paths.capacity() * std::mem::size_of::<PathBuf>(),
                |bytes, path| bytes.saturating_add(path.capacity()),
            ),
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
    ///
    /// # Panics
    /// When the endpoint generation space is exhausted. A generation must
    /// never repeat: doing so would make an event from an old endpoint
    /// indistinguishable from a current one.
    pub fn advance(&mut self) -> EndpointGeneration {
        self.generation = EndpointGeneration(
            self.generation
                .0
                .checked_add(1)
                .expect("input endpoint generation exhausted"),
        );
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

/// Bounded single-source inbox. It keeps stamped events in order, merges
/// adjacent pointer moves and wheel deltas, and hands an event back when full
/// rather than lose a key, button, focus or IME transition. It does not reject
/// events: generation, order and disconnects remain the router's decision at
/// drain, the same on every path. It only avoids coalescing metadata
/// regressions, so the router can still report those malformed events. Hosts
/// own any cross-thread wakeup.
#[derive(Debug)]
pub struct InputEndpoint {
    queue: VecDeque<CanonicalInputEvent>,
    max_events: usize,
    max_payload_bytes: usize,
    payload_bytes: usize,
}

impl InputEndpoint {
    pub fn new(max_events: usize, max_payload_bytes: usize) -> Self {
        Self {
            queue: VecDeque::with_capacity(max_events),
            max_events,
            max_payload_bytes,
            payload_bytes: 0,
        }
    }

    pub fn is_empty(&self) -> bool {
        self.queue.is_empty()
    }

    /// Queue `event`, merged into the last one when both are coalescible. A
    /// full queue returns the event: drain, then push it again. The byte
    /// budget bounds a burst, not one event: an empty endpoint takes an event
    /// larger than the whole budget (a drop of many files) on its own, so the
    /// retry after a drain always lands.
    #[allow(clippy::result_large_err)]
    pub fn push(&mut self, event: CanonicalInputEvent) -> Result<(), CanonicalInputEvent> {
        metric!(nana_diagnostics::framework::runtime::INPUT_EVENTS);
        if self
            .queue
            .back_mut()
            .is_some_and(|last| coalesce(last, &event))
        {
            metric!(nana_diagnostics::framework::runtime::INPUT_COALESCED);
            return Ok(());
        }
        let bytes = event.payload.allocation_bytes();
        if self.queue.len() >= self.max_events
            || (!self.queue.is_empty()
                && bytes > self.max_payload_bytes.saturating_sub(self.payload_bytes))
        {
            return Err(event);
        }
        metric!(
            nana_diagnostics::framework::runtime::INPUT_PAYLOAD_BYTES,
            bytes as u64
        );
        self.payload_bytes += bytes;
        self.queue.push_back(event);
        Ok(())
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
        // The endpoint deliberately leaves validation to the Runtime, but a
        // coalesced sample must not hide a sequence/timestamp regression from
        // that validator by overwriting the queued event's metadata.
        || next.metadata.sequence <= previous.metadata.sequence
        || next.metadata.timestamp < previous.metadata.timestamp
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
                pointer_id: PointerId(3),
                ..PointerInput::mouse(phase, sequence as f32, 2.0)
            }),
        }
    }

    fn drain(endpoint: &mut InputEndpoint) -> Vec<CanonicalInputEvent> {
        std::iter::from_fn(|| endpoint.pop()).collect()
    }

    #[test]
    fn adjacent_moves_coalesce_but_transitions_are_preserved() {
        let mut endpoint = InputEndpoint::new(8, 0);
        for (sequence, phase) in [
            (1, PointerPhase::Move),
            (2, PointerPhase::Move),
            (3, PointerPhase::Down),
        ] {
            endpoint.push(pointer(sequence, phase)).unwrap();
        }
        let queued = drain(&mut endpoint);
        assert_eq!(queued.len(), 2);
        assert!(matches!(
            queued[0].payload,
            InputPayload::Pointer(PointerInput { x: 2.0, .. })
        ));
    }

    #[test]
    fn high_frequency_moves_keep_one_queued_sample() {
        let mut endpoint = InputEndpoint::new(8, 0);
        for sequence in 1..=1_000 {
            endpoint
                .push(pointer(sequence, PointerPhase::Move))
                .unwrap();
        }
        assert_eq!(drain(&mut endpoint).len(), 1);
    }

    #[test]
    fn a_full_endpoint_returns_the_event_for_a_retry() {
        let mut endpoint = InputEndpoint::new(2, 1);
        let text = |sequence| CanonicalInputEvent {
            metadata: meta(sequence),
            payload: InputPayload::Text(CommittedText::new("hello")),
        };
        // Over the byte budget: alone in an empty endpoint it fits, behind
        // another event it waits for a drain.
        endpoint.push(text(1)).unwrap();
        assert_eq!(endpoint.push(text(2)), Err(text(2)));
        endpoint.pop();
        endpoint.push(text(2)).unwrap();
        endpoint.pop();

        let mut endpoint = InputEndpoint::new(1, 1);
        endpoint.push(pointer(2, PointerPhase::Down)).unwrap();
        let up = endpoint.push(pointer(3, PointerPhase::Up)).unwrap_err();
        endpoint.pop();
        endpoint.push(up).unwrap();
    }

    #[test]
    fn wheel_coalescing_preserves_target_position() {
        let mut endpoint = InputEndpoint::new(4, 0);
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
        for (sequence, x) in [(1, 1.0), (2, 1.0), (3, 2.0)] {
            endpoint.push(wheel(sequence, x)).unwrap();
        }
        let queued = drain(&mut endpoint);
        assert_eq!(queued.len(), 2);
        assert!(matches!(
            queued[0].payload,
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
        assert_eq!(sequencer.advance(), EndpointGeneration(2));
        let restarted = sequencer.stamp(DeviceId(1), Duration::ZERO);
        assert_eq!(restarted.sequence, InputSequence(1));
        assert_eq!(restarted.generation, EndpointGeneration(2));
    }

    #[test]
    #[should_panic(expected = "input endpoint generation exhausted")]
    fn advancing_an_exhausted_generation_panics() {
        let mut sequencer = InputSequencer::new(InputSourceId(4), EndpointGeneration(u64::MAX));
        sequencer.advance();
    }

    #[test]
    fn regressed_moves_are_not_coalesced() {
        let mut endpoint = InputEndpoint::new(4, 0);
        let mut first = pointer(2, PointerPhase::Move);
        first.metadata.timestamp = InputTimestamp(20);
        endpoint.push(first).unwrap();

        // A lower sequence must remain visible to the router as an out of
        // order event rather than replacing the queued sample.
        let mut sequence_regression = pointer(1, PointerPhase::Move);
        sequence_regression.metadata.timestamp = InputTimestamp(30);
        endpoint.push(sequence_regression).unwrap();

        // A timestamp regression is also invalid and must not be hidden by
        // coalescing.
        let mut equal_sequence = pointer(1, PointerPhase::Move);
        equal_sequence.metadata.timestamp = InputTimestamp(40);
        endpoint.push(equal_sequence).unwrap();

        let mut timestamp_regression = pointer(3, PointerPhase::Move);
        timestamp_regression.metadata.timestamp = InputTimestamp(10);
        endpoint.push(timestamp_regression).unwrap();

        assert_eq!(endpoint.queue.len(), 4);
        assert_eq!(endpoint.pop().unwrap().metadata.sequence, InputSequence(2));
        assert_eq!(endpoint.pop().unwrap().metadata.sequence, InputSequence(1));
        assert_eq!(endpoint.pop().unwrap().metadata.sequence, InputSequence(1));
        assert_eq!(endpoint.pop().unwrap().metadata.sequence, InputSequence(3));
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
