//! Backend-neutral motion contracts.
//!
//! This module is the **only** Motion timeline authority. Runtime
//! `AnimationSpec` is the timing / playback subset (id, target, start,
//! duration, frame interval, easing, iteration, direction, fill, pause) and
//! samples unit progress through [`evaluate_progress`]. Vue/CSS compilers
//! must emit [`MotionTrack`] / [`MotionGraph`], not a second clock.
//!
//! [`PresentationStore`] is a transient overlay over these tracks, not a second
//! timeline. Compositor-safe tracks (`AnimationClass::Compositor`) also compile
//! to a [`MotionDescriptor`] in a generational slab; CPU evaluation still goes
//! through [`evaluate_track`]. GPU storage-buffer packing lives in [`gpu`].

mod codec;
mod descriptor;
mod easing;
mod eval;
mod gpu;
mod inspector;
mod ir;
mod playback;
mod presentation;
mod property;

pub use codec::{
    MotionCodecError, MotionCodecId, MotionCodecInfo, MotionCodecRegistry, MotionValueKind,
};
pub use descriptor::{
    CompiledMotion, MOTION_DESCRIPTOR_VERSION, MotionDescriptor, MotionDescriptorError,
    MotionDescriptorStore, MotionHandle, PresentationSlot, compile_motion_descriptor,
    decode_motion_track, evaluate_descriptor,
};
pub use easing::Easing;
pub use eval::{
    ProgressSample, damped_harmonic, evaluate_progress, evaluate_track, evaluate_track_at,
    retarget_track,
};
pub use gpu::{
    MOTION_GPU_CURVE_DECAY, MOTION_GPU_CURVE_EASING, MOTION_GPU_CURVE_SPRING,
    MOTION_GPU_CURVE_STEPS, MOTION_GPU_DESCRIPTOR_SIZE, MOTION_GPU_EASING_BEZIER,
    MOTION_GPU_EASING_IN_OUT_CUBIC, MOTION_GPU_EASING_LINEAR, MOTION_GPU_EASING_OUT_CUBIC,
    MOTION_GPU_FLAG_LIVE, MOTION_GPU_KEYFRAME_SIZE, MOTION_GPU_KIND_COLOR,
    MOTION_GPU_KIND_DISCRETE, MOTION_GPU_KIND_SCALAR, MOTION_GPU_KIND_TRANSFORM,
    MOTION_GPU_TIME_SIZE, MOTION_GPU_VALUE_SIZE, MotionGpuDescriptor, MotionGpuKeyframe,
    MotionGpuTime, MotionGpuValue, as_bytes as motion_gpu_as_bytes,
    pack_descriptor as pack_gpu_descriptor, pack_keyframe as pack_gpu_keyframe,
    pack_property as pack_gpu_property, pack_value as pack_gpu_value,
    unpack_value as unpack_gpu_value,
};
pub use inspector::{
    MotionEvaluatorBackend, MotionInspectorEntry, MotionWorkCounters, cpu_fallback_reason,
};
pub use ir::{
    DecayParams, Keyframe, MotionCurve, MotionGraph, MotionInterrupt, MotionSample, MotionTargetId,
    MotionTo, MotionTrack, MotionTrackId, MotionValue, PresentationPair, Spring, SpringParams,
    StepJump, Timeline, track_completion_deadline,
};
pub use playback::{
    AnimationDirection, AnimationFillMode, AnimationIteration, AnimationPlayState,
    AnimationPlayback, MotionTiming, TimedProgress,
};
pub use presentation::{PresentationOverlay, PresentationStore};
pub use property::{
    AnimatableProperty, AnimationClass, FlipRect, classify_animatable_property,
    invert_flip_translate,
};

use std::time::Duration;

/// Shared motion durations aligned with the LiliaUI motion spec. Surfaces wire
/// these in per interaction; the constants only centralize the values.
pub const HOVER_COLOR: Duration = Duration::from_millis(120);
/// Overlay fade-in/out duration.
pub const OVERLAY_FADE: Duration = Duration::from_millis(140);
/// Menu opacity transition duration.
pub const MENU_OPACITY: Duration = Duration::from_millis(160);
/// Menu pop-in scale/translate duration.
pub const MENU_POP: Duration = Duration::from_millis(180);
/// Sidebar collapse/expand duration.
pub const SIDEBAR_COLLAPSE: Duration = Duration::from_millis(260);
/// Skeleton pulse cycle duration.
pub const SKELETON_PULSE: Duration = Duration::from_millis(1400);

/// One full turn of an indeterminate busy indicator.
pub const SPINNER_ROTATION: Duration = Duration::from_millis(900);
/// Button / switch / card loading indicator cycle.
pub const LOADING_SPIN: Duration = Duration::from_millis(800);
