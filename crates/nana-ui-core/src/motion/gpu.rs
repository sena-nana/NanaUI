//! GPU MotionDescriptor POD. Layout is the shader contract.
//!
//! Packed at start / retarget / cancel. Steady frames keep this blob and only
//! change the shared time uniform. Not a second evaluator: [`pack_descriptor`]
//! stores the same fields [`super::evaluate_descriptor`] reads.

use std::time::Duration;

use super::{
    descriptor::{MOTION_DESCRIPTOR_VERSION, MotionDescriptor},
    easing::Easing,
    ir::{Keyframe, MotionCurve, MotionValue, StepJump},
    playback::{AnimationDirection, AnimationFillMode, AnimationIteration, AnimationPlayState},
    property::AnimatableProperty,
};
use crate::PaintTransform;

pub const MOTION_GPU_VALUE_SIZE: usize = 48;
pub const MOTION_GPU_DESCRIPTOR_SIZE: usize = 272;
pub const MOTION_GPU_KEYFRAME_SIZE: usize = 80;
pub const MOTION_GPU_TIME_SIZE: usize = 16;

pub const MOTION_GPU_KIND_SCALAR: u32 = 0;
pub const MOTION_GPU_KIND_COLOR: u32 = 1;
pub const MOTION_GPU_KIND_TRANSFORM: u32 = 2;
pub const MOTION_GPU_KIND_DISCRETE: u32 = 3;

pub const MOTION_GPU_CURVE_EASING: u32 = 0;
pub const MOTION_GPU_CURVE_STEPS: u32 = 1;
pub const MOTION_GPU_CURVE_SPRING: u32 = 2;
pub const MOTION_GPU_CURVE_DECAY: u32 = 3;

pub const MOTION_GPU_EASING_LINEAR: u32 = 0;
pub const MOTION_GPU_EASING_OUT_CUBIC: u32 = 1;
pub const MOTION_GPU_EASING_IN_OUT_CUBIC: u32 = 2;
pub const MOTION_GPU_EASING_BEZIER: u32 = 3;

pub const MOTION_GPU_FLAG_LIVE: u32 = 1;

/// One interpolable value. `channels`/`extra` follow [`MotionValue`].
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MotionGpuValue {
    pub kind: u32,
    pub discrete: u32,
    pub _pad: [u32; 2],
    pub channels: [f32; 4],
    pub extra: [f32; 4],
}

impl MotionGpuValue {
    pub const fn vacant() -> Self {
        Self {
            kind: MOTION_GPU_KIND_SCALAR,
            discrete: 0,
            _pad: [0; 2],
            channels: [0.0; 4],
            extra: [0.0; 4],
        }
    }
}

/// Shared presentation clock. Product frames write this without touching
/// the descriptor storage buffer.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MotionGpuTime {
    pub now: f32,
    pub eval_motion_id: u32,
    pub _pad: [u32; 2],
}

impl MotionGpuTime {
    pub fn new(now: Duration) -> Self {
        Self {
            now: duration_secs(now),
            eval_motion_id: 0,
            _pad: [0; 2],
        }
    }

    pub fn with_eval(now: Duration, motion_id: u32) -> Self {
        Self {
            now: duration_secs(now),
            eval_motion_id: motion_id,
            _pad: [0; 2],
        }
    }
}

/// Storage-buffer keyframe. `easing_kind == u32::MAX` inherits the track curve.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MotionGpuKeyframe {
    pub offset: f32,
    pub easing_kind: u32,
    pub _pad: [u32; 2],
    pub bezier: [f32; 4],
    pub value: MotionGpuValue,
}

impl MotionGpuKeyframe {
    pub fn dummy() -> Self {
        Self {
            offset: 0.0,
            easing_kind: u32::MAX,
            _pad: [0; 2],
            bezier: [0.0; 4],
            value: MotionGpuValue::vacant(),
        }
    }
}

/// One slab slot. Index equals [`super::MotionHandle::index`].
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MotionGpuDescriptor {
    pub generation: u32,
    pub codec: u32,
    pub property: u32,
    pub flags: u32,
    pub track_lo: u32,
    pub track_hi: u32,
    pub target_lo: u32,
    pub target_hi: u32,
    pub start: f32,
    pub delay: f32,
    pub duration: f32,
    pub paused_at: f32,
    pub iteration: u32,
    pub direction: u32,
    pub fill: u32,
    pub play_state: u32,
    pub curve_kind: u32,
    pub easing_kind: u32,
    pub steps_count: u32,
    pub steps_jump: u32,
    pub bezier: [f32; 4],
    pub spring: [f32; 4],
    pub from: MotionGpuValue,
    pub to: MotionGpuValue,
    pub velocity: MotionGpuValue,
    pub keyframe_start: u32,
    pub keyframe_count: u32,
    pub version: u32,
    pub _end_pad: u32,
}

const _: () = assert!(std::mem::size_of::<MotionGpuValue>() == MOTION_GPU_VALUE_SIZE);
const _: () = assert!(std::mem::size_of::<MotionGpuDescriptor>() == MOTION_GPU_DESCRIPTOR_SIZE);
const _: () = assert!(std::mem::size_of::<MotionGpuKeyframe>() == MOTION_GPU_KEYFRAME_SIZE);
const _: () = assert!(std::mem::size_of::<MotionGpuTime>() == MOTION_GPU_TIME_SIZE);
const _: () = assert!(std::mem::align_of::<MotionGpuDescriptor>() <= 16);

impl MotionGpuDescriptor {
    pub fn vacant(generation: u32) -> Self {
        Self {
            generation,
            codec: 0,
            property: 0,
            flags: 0,
            track_lo: 0,
            track_hi: 0,
            target_lo: 0,
            target_hi: 0,
            start: 0.0,
            delay: 0.0,
            duration: 0.0,
            paused_at: -1.0,
            iteration: 1,
            direction: 0,
            fill: 0,
            play_state: 0,
            curve_kind: 0,
            easing_kind: 0,
            steps_count: 0,
            steps_jump: 0,
            bezier: [0.0; 4],
            spring: [0.0; 4],
            from: MotionGpuValue::vacant(),
            to: MotionGpuValue::vacant(),
            velocity: MotionGpuValue::vacant(),
            keyframe_start: 0,
            keyframe_count: 0,
            version: u32::from(MOTION_DESCRIPTOR_VERSION),
            _end_pad: 0,
        }
    }

    pub fn is_live(self) -> bool {
        self.flags & MOTION_GPU_FLAG_LIVE != 0
    }
}

pub fn pack_value(value: MotionValue) -> MotionGpuValue {
    match value {
        MotionValue::Scalar(v) => MotionGpuValue {
            kind: MOTION_GPU_KIND_SCALAR,
            discrete: 0,
            _pad: [0; 2],
            channels: [v, 0.0, 0.0, 0.0],
            extra: [0.0; 4],
        },
        MotionValue::Color(c) => MotionGpuValue {
            kind: MOTION_GPU_KIND_COLOR,
            discrete: 0,
            _pad: [0; 2],
            channels: c,
            extra: [0.0; 4],
        },
        MotionValue::Transform(p) => MotionGpuValue {
            kind: MOTION_GPU_KIND_TRANSFORM,
            discrete: 0,
            _pad: [0; 2],
            channels: [p.a, p.b, p.c, p.d],
            extra: [p.e, p.f, 0.0, 0.0],
        },
        MotionValue::Discrete(tag) => MotionGpuValue {
            kind: MOTION_GPU_KIND_DISCRETE,
            discrete: tag,
            _pad: [0; 2],
            channels: [0.0; 4],
            extra: [0.0; 4],
        },
    }
}

pub fn unpack_value(value: MotionGpuValue) -> MotionValue {
    match value.kind {
        MOTION_GPU_KIND_COLOR => MotionValue::Color(value.channels),
        MOTION_GPU_KIND_TRANSFORM => MotionValue::Transform(PaintTransform {
            a: value.channels[0],
            b: value.channels[1],
            c: value.channels[2],
            d: value.channels[3],
            e: value.extra[0],
            f: value.extra[1],
        }),
        MOTION_GPU_KIND_DISCRETE => MotionValue::Discrete(value.discrete),
        _ => MotionValue::Scalar(value.channels[0]),
    }
}

pub fn pack_property(property: AnimatableProperty) -> u32 {
    match property {
        AnimatableProperty::Transform => 0,
        AnimatableProperty::Opacity => 1,
        AnimatableProperty::Clip => 2,
        AnimatableProperty::Color => 3,
        AnimatableProperty::Background => 4,
        AnimatableProperty::Blur => 5,
        AnimatableProperty::Filter => 6,
        AnimatableProperty::Shadow => 7,
        AnimatableProperty::ShaderParameter => 8,
        AnimatableProperty::Width => 9,
        AnimatableProperty::Height => 10,
        AnimatableProperty::Padding => 11,
        AnimatableProperty::Margin => 12,
        AnimatableProperty::FontSize => 13,
        AnimatableProperty::FontAxis(_) => 14,
        AnimatableProperty::Display => 15,
        AnimatableProperty::Progress => 16,
    }
}

pub fn pack_keyframe(stop: &Keyframe) -> MotionGpuKeyframe {
    let (easing_kind, bezier) = match stop.easing {
        None => (u32::MAX, [0.0; 4]),
        Some(easing) => pack_easing(easing),
    };
    MotionGpuKeyframe {
        offset: stop.offset,
        easing_kind,
        _pad: [0; 2],
        bezier,
        value: pack_value(stop.value),
    }
}

pub fn pack_descriptor(
    descriptor: &MotionDescriptor,
    generation: u32,
    gpu_keyframe_start: u32,
) -> MotionGpuDescriptor {
    let (curve_kind, easing_kind, steps_count, steps_jump, bezier, spring) =
        pack_curve(descriptor.curve);
    let track = descriptor.track_id.get();
    let target = descriptor.slot.target.get();
    let paused_at = match descriptor.playback.paused_at {
        Some(at) => duration_secs(at),
        None => -1.0,
    };
    let iteration = match descriptor.playback.iteration_count {
        AnimationIteration::Infinite => 0,
        AnimationIteration::Count(count) => count,
    };
    MotionGpuDescriptor {
        generation,
        codec: u32::from(descriptor.codec.get()),
        property: pack_property(descriptor.property),
        flags: MOTION_GPU_FLAG_LIVE,
        track_lo: track as u32,
        track_hi: (track >> 32) as u32,
        target_lo: target as u32,
        target_hi: (target >> 32) as u32,
        start: duration_secs(descriptor.timing.start),
        delay: duration_secs(descriptor.timing.delay),
        duration: duration_secs(descriptor.timing.duration),
        paused_at,
        iteration,
        direction: pack_direction(descriptor.playback.direction),
        fill: pack_fill(descriptor.playback.fill_mode),
        play_state: match descriptor.playback.play_state {
            AnimationPlayState::Running => 0,
            AnimationPlayState::Paused => 1,
        },
        curve_kind,
        easing_kind,
        steps_count,
        steps_jump,
        bezier,
        spring,
        from: pack_value(descriptor.from),
        to: pack_value(descriptor.to),
        velocity: pack_value(descriptor.velocity),
        keyframe_start: gpu_keyframe_start,
        keyframe_count: descriptor.keyframe_count,
        version: u32::from(descriptor.version),
        _end_pad: 0,
    }
}

pub fn duration_secs(duration: Duration) -> f32 {
    duration.as_secs_f32()
}

pub fn as_bytes<T>(slice: &[T]) -> &[u8] {
    let len = std::mem::size_of_val(slice);
    if len == 0 {
        return &[];
    }
    unsafe { std::slice::from_raw_parts(slice.as_ptr().cast::<u8>(), len) }
}

fn pack_curve(curve: MotionCurve) -> (u32, u32, u32, u32, [f32; 4], [f32; 4]) {
    match curve {
        MotionCurve::Easing(easing) => {
            let (kind, bezier) = pack_easing(easing);
            (MOTION_GPU_CURVE_EASING, kind, 0, 0, bezier, [0.0; 4])
        }
        MotionCurve::Steps { count, jump } => (
            MOTION_GPU_CURVE_STEPS,
            MOTION_GPU_EASING_LINEAR,
            count,
            pack_jump(jump),
            [0.0; 4],
            [0.0; 4],
        ),
        MotionCurve::Spring(params) => (
            MOTION_GPU_CURVE_SPRING,
            MOTION_GPU_EASING_LINEAR,
            0,
            0,
            [0.0; 4],
            [params.stiffness, params.damping, params.mass, 0.0],
        ),
        MotionCurve::Decay(params) => (
            MOTION_GPU_CURVE_DECAY,
            MOTION_GPU_EASING_LINEAR,
            0,
            0,
            [0.0; 4],
            [0.0, 0.0, 0.0, params.time_constant],
        ),
    }
}

fn pack_easing(easing: Easing) -> (u32, [f32; 4]) {
    match easing {
        Easing::Linear => (MOTION_GPU_EASING_LINEAR, [0.0; 4]),
        Easing::EaseOutCubic => (MOTION_GPU_EASING_OUT_CUBIC, [0.0; 4]),
        Easing::EaseInOutCubic => (MOTION_GPU_EASING_IN_OUT_CUBIC, [0.0; 4]),
        Easing::CubicBezier(points) => (MOTION_GPU_EASING_BEZIER, points),
    }
}

fn pack_jump(jump: StepJump) -> u32 {
    match jump {
        StepJump::Start => 0,
        StepJump::End => 1,
        StepJump::None => 2,
        StepJump::Both => 3,
    }
}

fn pack_direction(direction: AnimationDirection) -> u32 {
    match direction {
        AnimationDirection::Normal => 0,
        AnimationDirection::Reverse => 1,
        AnimationDirection::Alternate => 2,
        AnimationDirection::AlternateReverse => 3,
    }
}

fn pack_fill(fill: AnimationFillMode) -> u32 {
    match fill {
        AnimationFillMode::None => 0,
        AnimationFillMode::Forwards => 1,
        AnimationFillMode::Backwards => 2,
        AnimationFillMode::Both => 3,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::motion::{
        AnimationPlayback, MotionCodecId, MotionCodecRegistry, MotionTargetId, MotionTiming,
        MotionTo, MotionTrack, MotionTrackId, compile_motion_descriptor,
    };

    #[test]
    fn packed_sizes_match_the_shader_contract() {
        assert_eq!(
            std::mem::size_of::<MotionGpuDescriptor>(),
            MOTION_GPU_DESCRIPTOR_SIZE
        );
        assert_eq!(
            std::mem::size_of::<MotionGpuKeyframe>(),
            MOTION_GPU_KEYFRAME_SIZE
        );
        assert_eq!(std::mem::size_of::<MotionGpuTime>(), MOTION_GPU_TIME_SIZE);
        assert_eq!(MOTION_GPU_DESCRIPTOR_SIZE % 16, 0);
        assert_eq!(MOTION_GPU_KEYFRAME_SIZE % 16, 0);
    }

    #[test]
    fn width_is_not_a_transform_gpu_codec() {
        assert_ne!(
            pack_property(AnimatableProperty::Width),
            pack_property(AnimatableProperty::Transform)
        );
        assert_eq!(u32::from(MotionCodecId::TRANSFORM.get()), 2);
        assert_eq!(MotionCodecId::for_property(AnimatableProperty::Width), None);
    }

    #[test]
    fn opacity_and_transform_round_trip_values() {
        let scalar = pack_value(MotionValue::Scalar(0.35));
        match unpack_value(scalar) {
            MotionValue::Scalar(v) => assert!((v - 0.35).abs() < 1e-6),
            other => panic!("{other:?}"),
        }
        let from = PaintTransform {
            a: 1.0,
            b: 0.0,
            c: 0.0,
            d: 1.0,
            e: 40.0,
            f: 8.0,
        };
        match unpack_value(pack_value(MotionValue::Transform(from))) {
            MotionValue::Transform(got) => {
                assert!((got.e - 40.0).abs() < 1e-6);
                assert!((got.f - 8.0).abs() < 1e-6);
            }
            other => panic!("{other:?}"),
        }
        let color = pack_value(MotionValue::Color([0.1, 0.2, 0.3, 1.0]));
        match unpack_value(color) {
            MotionValue::Color(c) => assert!((c[1] - 0.2).abs() < 1e-6),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn compile_then_pack_is_live_and_typed() {
        let registry = MotionCodecRegistry::builtin();
        let track = MotionTrack::transition(
            MotionTrackId::new(1).unwrap(),
            MotionTargetId::new(1).unwrap(),
            AnimatableProperty::Opacity,
            MotionValue::Scalar(0.0),
            MotionValue::Scalar(1.0),
            MotionTiming::new(
                Duration::ZERO,
                Duration::from_millis(100),
                Duration::from_millis(16),
            ),
            MotionCurve::Easing(Easing::Linear),
            AnimationPlayback::default(),
        );
        let compiled = compile_motion_descriptor(&track, &registry, None).expect("compile");
        let packed = pack_descriptor(&compiled.descriptor, 3, 0);
        assert!(packed.is_live());
        assert_eq!(packed.generation, 3);
        assert_eq!(packed.codec, u32::from(MotionCodecId::OPACITY.get()));
        assert_eq!(packed.property, pack_property(AnimatableProperty::Opacity));
        assert_eq!(packed.curve_kind, MOTION_GPU_CURVE_EASING);
        match compiled.descriptor.to {
            MotionValue::Scalar(v) => assert!((v - 1.0).abs() < 1e-6),
            other => panic!("{other:?}"),
        }
        assert!(matches!(track.to, MotionTo::Value(_)));
    }
}
