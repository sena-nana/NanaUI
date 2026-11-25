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
mod lowering;

pub use canonical::{
    CanonicalInputEvent, CommittedText, CompositionInput, DeviceId, EndpointGeneration,
    FileDragInput, FocusDirection, InputEndpoint, InputMetadata, InputPayload, InputSequence,
    InputSequencer, InputSourceId, InputTimestamp, KeyInput, KeyState, LogicalKey, NavigationInput,
    NavigationIntent, NavigationSource, PhysicalKey, PointerId, PointerInput, WheelInput,
    WheelUnit,
};
pub use host_services::{
    CursorIcon, HeadlessHostServices, HostServiceError, HostServices, SurroundingText,
    TextInputContext, TextInputPurpose, UnsupportedHostServices,
};
pub use input::{InputDisposition, InputModifiers, PointerPhase, PointerType};
pub use lowering::{
    DragTransferTracker, PRIMARY_MOUSE_BUTTON, PenFramePatch, PointerDescriptor, PointerState,
    ScreenSpace, gamepad_dpad_navigation, held_mouse_button, mouse_button_mask, screen_position,
    shift_wheel_axes,
};
pub use nana_ui_core::{
    DEFAULT_MAX_TRANSFER_BYTES, DataFormat, DataFormatError, DataTransfer, DragCancelReason,
    DragSessionId, DropAccepts, DropEffect, DropEffects, DropKind, FileDragKind,
    InternalPayloadToken, LazyFnProvider, MAX_MIME_LEN, TransferAction, TransferCachePolicy,
    TransferEntry, TransferEntryMeta, TransferError, TransferImage, TransferLifetime,
    TransferOrigin, TransferProvider, file_uri_to_path, format_uri_list, parse_uri_list,
    path_to_file_uri,
};
