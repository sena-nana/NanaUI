//! Canonical input: the events every host lowers its native input to, the
//! bounded endpoint that carries them, and the host services contract the
//! Runtime drives while it routes them.
//!
//! This crate has no optional or platform dependencies, so the Runtime can
//! speak the contract without reaching any platform backend.
//! `nana-ui-platform` re-exports all of it.

mod canonical;
mod host_services;
mod input;

pub use canonical::{
    CanonicalInputEvent, CommittedText, CompositionInput, DeviceId, EndpointGeneration,
    InputDeviceCounters, InputEndpoint, InputEndpointCounters, InputEnqueueOutcome, InputMetadata,
    InputPayload, InputRejection, InputSequence, InputSequencer, InputSourceId, InputTimestamp,
    KeyInput, KeyState, LogicalKey, PhysicalKey, PointerId, PointerInput, RejectedInput,
    WheelInput, WheelUnit,
};
pub use host_services::{
    CursorIcon, HeadlessHostServices, HostServiceCounters, HostServiceError, HostServices,
    SurroundingText, TextInputContext, TextInputPurpose, UnsupportedHostServices,
};
pub use input::{InputDisposition, InputModifiers, PointerPhase, PointerType};
