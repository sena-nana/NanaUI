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
pub use presentation::{MotionLayer, PresentationOverlay, PresentationStore};
pub use property::{
    AnimatableProperty, AnimationClass, FlipRect, classify_animatable_property,
    invert_flip_translate, is_font_variation_settings,
};

use std::time::Duration;

/// The **default theme's** motion durations.
///
/// These are a view of [`MotionTokens::DEFAULT`], not a second authority: the
/// numbers live on the theme, and each constant reads its role. A surface that
/// can reach an installed theme should ask it —
/// `theme.duration(MotionRole::HoverColor)` — because only that follows a
/// theme that moved the value. Issue #101 §1.4 F2 is why the direction matters:
/// when the durations were the authority and the theme merely had two unread
/// `motion_*_ms` fields, no theme could change a transition at all.
const DEFAULT_MOTION: crate::theme::MotionTokens = crate::theme::MotionTokens::DEFAULT;

/// Hover / pressed colour cross-fade on a control.
pub const HOVER_COLOR: Duration = DEFAULT_MOTION.duration(crate::theme::MotionRole::HoverColor);
/// Overlay fade-in/out duration.
pub const OVERLAY_FADE: Duration = DEFAULT_MOTION.duration(crate::theme::MotionRole::OverlayFade);
/// Menu opacity transition duration.
pub const MENU_OPACITY: Duration = DEFAULT_MOTION.duration(crate::theme::MotionRole::MenuOpacity);
/// Menu pop-in scale/translate duration.
pub const MENU_POP: Duration = DEFAULT_MOTION.duration(crate::theme::MotionRole::MenuPop);
/// Sidebar collapse/expand duration.
pub const SIDEBAR_COLLAPSE: Duration =
    DEFAULT_MOTION.duration(crate::theme::MotionRole::SidebarCollapse);
/// Skeleton pulse cycle duration.
pub const SKELETON_PULSE: Duration =
    DEFAULT_MOTION.duration(crate::theme::MotionRole::SkeletonPulse);

/// One full turn of an indeterminate busy indicator.
pub const SPINNER_ROTATION: Duration =
    DEFAULT_MOTION.duration(crate::theme::MotionRole::SpinnerRotation);
/// Button / switch / card loading indicator cycle.
pub const LOADING_SPIN: Duration = DEFAULT_MOTION.duration(crate::theme::MotionRole::LoadingSpin);
