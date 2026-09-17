//! Versioned MotionDescriptor contract and generational slab.
//!
//! Compile a compositor [`crate::motion::MotionTrack`] once; steady frames
//! only change the shared timestamp. Evaluation decodes back to a track and
//! calls [`crate::motion::evaluate_track`] — this is not a second timeline.
//!
//! GPU storage packing is [`super::gpu`]; this module stays the CPU source of
//! truth for that layout.

use std::{collections::HashMap, time::Duration};

use super::{
    codec::{MotionCodecId, MotionCodecRegistry},
    eval::evaluate_track,
    ir::{
        Keyframe, MotionSample, MotionTargetId, MotionTo, MotionTrack, MotionTrackId, MotionValue,
    },
    playback::{AnimationPlayback, MotionTiming},
    property::{AnimatableProperty, AnimationClass},
};

/// Packed descriptor layout version. Bump when field meaning changes.
pub const MOTION_DESCRIPTOR_VERSION: u16 = 1;

/// Generational slab index. A freed slot may be reused; the old generation
/// must not observe the new occupant.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct MotionHandle {
    index: u32,
    generation: u32,
}

impl MotionHandle {
    pub const NULL: Self = Self {
        index: 0,
        generation: 0,
    };

    pub const fn from_parts(index: u32, generation: u32) -> Self {
        Self { index, generation }
    }

    pub const fn index(self) -> u32 {
        self.index
    }

    pub const fn generation(self) -> u32 {
        self.generation
    }

    pub const fn is_null(self) -> bool {
        self.generation == 0
    }
}

/// `(target, property)` presentation slot the descriptor writes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct PresentationSlot {
    pub target: MotionTargetId,
    pub property: AnimatableProperty,
}

/// Versioned compositor descriptor. Semantic fields match Issue #87 §11.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MotionDescriptor {
    pub version: u16,
    pub codec: MotionCodecId,
    pub property: AnimatableProperty,
    pub track_id: MotionTrackId,
    pub slot: PresentationSlot,
    pub timing: MotionTiming,
    pub playback: AnimationPlayback,
    pub curve: super::ir::MotionCurve,
    pub from: MotionValue,
    pub to: MotionValue,
    pub velocity: MotionValue,
    pub keyframe_start: u32,
    pub keyframe_count: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MotionDescriptorError {
    NotCompositor,
    UnregisteredCodec,
    InvalidTrack,
    StaleHandle,
}

/// One compiled descriptor plus out-of-line keyframes (GPU range later).
#[derive(Debug, Clone, PartialEq)]
pub struct CompiledMotion {
    pub descriptor: MotionDescriptor,
    pub keyframes: Vec<Keyframe>,
}

fn compositor_codec(
    property: AnimatableProperty,
    registry: &MotionCodecRegistry,
    shader_codec: Option<MotionCodecId>,
) -> Result<MotionCodecId, MotionDescriptorError> {
    if property.animation_class() != AnimationClass::Compositor {
        return Err(MotionDescriptorError::NotCompositor);
    }
    if property == AnimatableProperty::ShaderParameter {
        let id = shader_codec.ok_or(MotionDescriptorError::UnregisteredCodec)?;
        let info = registry
            .get(id)
            .ok_or(MotionDescriptorError::UnregisteredCodec)?;
        if info.id.get() < MotionCodecId::CUSTOM_START {
            return Err(MotionDescriptorError::UnregisteredCodec);
        }
        return Ok(id);
    }
    registry
        .get(MotionCodecId::for_property(property).ok_or(MotionDescriptorError::NotCompositor)?)
        .map(|info| info.id)
        .ok_or(MotionDescriptorError::NotCompositor)
}

pub fn compile_motion_descriptor(
    track: &MotionTrack,
    registry: &MotionCodecRegistry,
    shader_codec: Option<MotionCodecId>,
) -> Result<CompiledMotion, MotionDescriptorError> {
    if track.execution_class() != AnimationClass::Compositor {
        return Err(MotionDescriptorError::NotCompositor);
    }
    if !track.is_valid() {
        return Err(MotionDescriptorError::InvalidTrack);
    }
    let codec = compositor_codec(track.property, registry, shader_codec)?;
    let (to, keyframes) = match &track.to {
        MotionTo::Value(value) => (*value, Vec::new()),
        MotionTo::Keyframes(stops) => (track.rest_value(), stops.clone()),
    };
    let keyframe_count = u32::try_from(keyframes.len()).expect("keyframe count fits u32");
    Ok(CompiledMotion {
        descriptor: MotionDescriptor {
            version: MOTION_DESCRIPTOR_VERSION,
            codec,
            property: track.property,
            track_id: track.id,
            slot: PresentationSlot {
                target: track.target,
                property: track.property,
            },
            timing: track.timing,
            playback: track.playback,
            curve: track.curve,
            from: track.from,
            to,
            velocity: track.velocity,
            keyframe_start: 0,
            keyframe_count,
        },
        keyframes,
    })
}

pub fn decode_motion_track(descriptor: &MotionDescriptor, keyframes: &[Keyframe]) -> MotionTrack {
    let to = if descriptor.keyframe_count == 0 {
        MotionTo::Value(descriptor.to)
    } else {
        let count = descriptor.keyframe_count as usize;
        let start = descriptor.keyframe_start as usize;
        let end = start.saturating_add(count).min(keyframes.len());
        let start = start.min(end);
        MotionTo::Keyframes(keyframes[start..end].to_vec())
    };
    MotionTrack {
        id: descriptor.track_id,
        target: descriptor.slot.target,
        property: descriptor.property,
        from: descriptor.from,
        to,
        timing: descriptor.timing,
        curve: descriptor.curve,
        playback: descriptor.playback,
        velocity: descriptor.velocity,
    }
}

/// Evaluate the compiled descriptor at `now` through [`evaluate_track`].
pub fn evaluate_descriptor(
    descriptor: &MotionDescriptor,
    keyframes: &[Keyframe],
    now: Duration,
) -> MotionSample {
    evaluate_track(&decode_motion_track(descriptor, keyframes), now)
}

#[derive(Debug, Clone)]
struct Slot {
    generation: u32,
    live: Option<Box<Occupied>>,
}

#[derive(Debug, Clone)]
struct Occupied {
    descriptor: MotionDescriptor,
    keyframes: Vec<Keyframe>,
}

impl Slot {
    fn vacant(generation: u32) -> Self {
        Self {
            generation,
            live: None,
        }
    }

    fn occupied(generation: u32, compiled: CompiledMotion) -> Self {
        let mut descriptor = compiled.descriptor;
        descriptor.keyframe_start = 0;
        descriptor.keyframe_count =
            u32::try_from(compiled.keyframes.len()).expect("keyframe count fits u32");
        Self {
            generation,
            live: Some(Box::new(Occupied {
                descriptor,
                keyframes: compiled.keyframes,
            })),
        }
    }
}

/// Generational slab of compositor descriptors. Start / retarget / cancel
/// mutate it; timestamp evaluation does not.
#[derive(Debug)]
pub struct MotionDescriptorStore {
    slots: Vec<Slot>,
    free: Vec<u32>,
    by_track: HashMap<MotionTrackId, MotionHandle>,
    registry: MotionCodecRegistry,
    structure_epoch: u64,
    source: u64,
}

fn next_store_source() -> u64 {
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
    NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
}

/// A clone evolves independently, so it is a different descriptor source.
impl Clone for MotionDescriptorStore {
    fn clone(&self) -> Self {
        Self {
            slots: self.slots.clone(),
            free: self.free.clone(),
            by_track: self.by_track.clone(),
            registry: self.registry.clone(),
            structure_epoch: self.structure_epoch,
            source: next_store_source(),
        }
    }
}

impl Default for MotionDescriptorStore {
    fn default() -> Self {
        Self::new()
    }
}

impl MotionDescriptorStore {
    pub fn new() -> Self {
        Self {
            slots: Vec::new(),
            free: Vec::new(),
            by_track: HashMap::new(),
            registry: MotionCodecRegistry::builtin(),
            structure_epoch: 0,
            source: next_store_source(),
        }
    }

    pub fn registry(&self) -> &MotionCodecRegistry {
        &self.registry
    }

    pub fn registry_mut(&mut self) -> &mut MotionCodecRegistry {
        &mut self.registry
    }

    /// Increments on alloc / in-place update / free. Steady evaluation must
    /// leave this unchanged.
    pub fn structure_epoch(&self) -> u64 {
        self.structure_epoch
    }

    /// Process-unique identity of this store. Epochs of different stores (one
    /// per window document) count independently, so consumers that cache a
    /// packed table across documents key it by `(source, structure_epoch)`.
    pub fn source(&self) -> u64 {
        self.source
    }

    pub fn slot_capacity(&self) -> usize {
        self.slots.len()
    }

    pub fn live_len(&self) -> usize {
        self.by_track.len()
    }

    pub fn is_empty(&self) -> bool {
        self.by_track.is_empty()
    }

    pub fn handle_for(&self, id: MotionTrackId) -> Option<MotionHandle> {
        self.by_track.get(&id).copied()
    }

    pub fn get(&self, handle: MotionHandle) -> Option<&MotionDescriptor> {
        let slot = self.slots.get(handle.index as usize)?;
        if slot.generation != handle.generation {
            return None;
        }
        Some(&slot.live.as_ref()?.descriptor)
    }

    pub fn keyframes(&self, handle: MotionHandle) -> Option<&[Keyframe]> {
        let slot = self.slots.get(handle.index as usize)?;
        if slot.generation != handle.generation {
            return None;
        }
        Some(slot.live.as_ref()?.keyframes.as_slice())
    }

    /// Insert or replace by track id. Same id keeps the handle generation.
    pub fn bind(&mut self, track: &MotionTrack) -> Result<MotionHandle, MotionDescriptorError> {
        self.bind_with_codec(track, None)
    }

    pub fn bind_shader(
        &mut self,
        track: &MotionTrack,
        codec: MotionCodecId,
    ) -> Result<MotionHandle, MotionDescriptorError> {
        self.bind_with_codec(track, Some(codec))
    }

    fn bind_with_codec(
        &mut self,
        track: &MotionTrack,
        shader_codec: Option<MotionCodecId>,
    ) -> Result<MotionHandle, MotionDescriptorError> {
        let compiled = compile_motion_descriptor(track, &self.registry, shader_codec)?;
        if let Some(handle) = self.by_track.get(&track.id).copied() {
            self.write_occupied(handle, compiled)?;
            self.bump_epoch();
            return Ok(handle);
        }
        let handle = self.alloc(compiled);
        self.by_track.insert(track.id, handle);
        self.bump_epoch();
        Ok(handle)
    }

    pub fn cancel(&mut self, id: MotionTrackId) -> bool {
        let Some(handle) = self.by_track.remove(&id) else {
            return false;
        };
        self.free(handle)
    }

    pub fn cancel_handle(&mut self, handle: MotionHandle) -> bool {
        if let Some(descriptor) = self.get(handle).copied() {
            self.by_track.remove(&descriptor.track_id);
        }
        self.free(handle)
    }

    pub fn cancel_target(&mut self, target: MotionTargetId) -> usize {
        let ids = self
            .by_track
            .iter()
            .filter_map(|(id, handle)| {
                self.get(*handle)
                    .filter(|descriptor| descriptor.slot.target == target)
                    .map(|_| *id)
            })
            .collect::<Vec<_>>();
        ids.into_iter().filter(|id| self.cancel(*id)).count()
    }

    pub fn evaluate(&self, handle: MotionHandle, now: Duration) -> Option<MotionSample> {
        let slot = self.slots.get(handle.index as usize)?;
        if slot.generation != handle.generation {
            return None;
        }
        let live = slot.live.as_ref()?;
        Some(evaluate_descriptor(&live.descriptor, &live.keyframes, now))
    }

    pub fn evaluate_track(&self, id: MotionTrackId, now: Duration) -> Option<MotionSample> {
        self.evaluate(self.handle_for(id)?, now)
    }

    /// Pack the slab for a GPU storage buffer. Index equals handle index,
    /// including vacant slots so a stale generation cannot observe a neighbor.
    pub fn pack_gpu(
        &self,
    ) -> (
        Vec<super::gpu::MotionGpuDescriptor>,
        Vec<super::gpu::MotionGpuKeyframe>,
    ) {
        use super::gpu::{MotionGpuDescriptor, MotionGpuKeyframe, pack_descriptor, pack_keyframe};
        let mut keyframes = Vec::new();
        let mut descriptors = Vec::with_capacity(self.slots.len().max(1));
        for slot in &self.slots {
            match slot.live.as_ref() {
                None => descriptors.push(MotionGpuDescriptor::vacant(slot.generation)),
                Some(live) => {
                    let start = u32::try_from(keyframes.len()).expect("keyframe count fits u32");
                    keyframes.extend(live.keyframes.iter().map(pack_keyframe));
                    descriptors.push(pack_descriptor(&live.descriptor, slot.generation, start));
                }
            }
        }
        if descriptors.is_empty() {
            descriptors.push(MotionGpuDescriptor::vacant(0));
        }
        if keyframes.is_empty() {
            keyframes.push(MotionGpuKeyframe::dummy());
        }
        (descriptors, keyframes)
    }

    fn alloc(&mut self, compiled: CompiledMotion) -> MotionHandle {
        if let Some(index) = self.free.pop() {
            let generation = self.slots[index as usize].generation;
            self.slots[index as usize] = Slot::occupied(generation, compiled);
            MotionHandle { index, generation }
        } else {
            let index = u32::try_from(self.slots.len()).expect("descriptor slab fits u32");
            self.slots.push(Slot::occupied(1, compiled));
            MotionHandle {
                index,
                generation: 1,
            }
        }
    }

    fn write_occupied(
        &mut self,
        handle: MotionHandle,
        compiled: CompiledMotion,
    ) -> Result<(), MotionDescriptorError> {
        let Some(slot) = self.slots.get_mut(handle.index as usize) else {
            return Err(MotionDescriptorError::StaleHandle);
        };
        if slot.generation != handle.generation || slot.live.is_none() {
            return Err(MotionDescriptorError::StaleHandle);
        }
        *slot = Slot::occupied(handle.generation, compiled);
        Ok(())
    }

    fn free(&mut self, handle: MotionHandle) -> bool {
        let Some(slot) = self.slots.get_mut(handle.index as usize) else {
            return false;
        };
        if slot.generation != handle.generation || slot.live.is_none() {
            return false;
        }
        *slot = Slot::vacant(next_generation(handle.generation));
        self.free.push(handle.index);
        self.bump_epoch();
        true
    }

    fn bump_epoch(&mut self) {
        self.structure_epoch = self.structure_epoch.wrapping_add(1);
    }
}

fn next_generation(current: u32) -> u32 {
    match current.wrapping_add(1) {
        0 => 1,
        next => next,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        PaintTransform,
        motion::{
            AnimationClass, AnimationFillMode, Easing, MotionCurve, SpringParams,
            codec::MotionValueKind, evaluate_track as eval_track,
        },
    };

    fn id(n: u64) -> MotionTrackId {
        MotionTrackId::new(n).unwrap()
    }

    fn target(n: u64) -> MotionTargetId {
        MotionTargetId::new(n).unwrap()
    }

    fn opacity_track(track_id: u64, from: f32, to: f32, curve: MotionCurve) -> MotionTrack {
        MotionTrack::transition(
            id(track_id),
            target(1),
            AnimatableProperty::Opacity,
            MotionValue::Scalar(from),
            MotionValue::Scalar(to),
            MotionTiming::new(
                Duration::ZERO,
                Duration::from_millis(100),
                Duration::from_millis(16),
            ),
            curve,
            AnimationPlayback::default(),
        )
    }

    fn transform_track(from: PaintTransform, to: PaintTransform) -> MotionTrack {
        MotionTrack::transition(
            id(1),
            target(1),
            AnimatableProperty::Transform,
            MotionValue::Transform(from),
            MotionValue::Transform(to),
            MotionTiming::new(
                Duration::ZERO,
                Duration::from_millis(100),
                Duration::from_millis(16),
            ),
            MotionCurve::Easing(Easing::Linear),
            AnimationPlayback::default(),
        )
    }

    fn assert_same_sample(a: &MotionSample, b: &MotionSample) {
        assert_eq!(a.property, b.property);
        assert_eq!(a.finished, b.finished);
        assert_eq!(a.applies, b.applies);
        assert!((a.progress - b.progress).abs() < 1e-6);
        match (a.value, b.value) {
            (MotionValue::Scalar(x), MotionValue::Scalar(y)) => assert!((x - y).abs() < 1e-5),
            (MotionValue::Transform(x), MotionValue::Transform(y)) => {
                for (l, r) in [x.a, x.b, x.c, x.d, x.e, x.f]
                    .into_iter()
                    .zip([y.a, y.b, y.c, y.d, y.e, y.f])
                {
                    assert!((l - r).abs() < 1e-5);
                }
            }
            (left, right) => assert_eq!(left, right),
        }
    }

    #[test]
    fn compile_decode_matches_evaluate_track_for_opacity_bezier_and_spring() {
        let registry = MotionCodecRegistry::builtin();
        let bezier = opacity_track(
            1,
            0.0,
            1.0,
            MotionCurve::Easing(Easing::CubicBezier([0.2, 0.8, 0.2, 1.0])),
        );
        let spring = opacity_track(
            2,
            0.0,
            1.0,
            MotionCurve::Spring(SpringParams::new(170.0, 26.0, 1.0)),
        );
        for track in [&bezier, &spring] {
            let compiled = compile_motion_descriptor(track, &registry, None).expect("compile");
            assert_eq!(compiled.descriptor.version, MOTION_DESCRIPTOR_VERSION);
            assert_eq!(compiled.descriptor.codec, MotionCodecId::OPACITY);
            assert_eq!(
                compiled.descriptor.slot.property,
                AnimatableProperty::Opacity
            );
            for ms in [0, 16, 40, 50, 80, 100, 200] {
                let now = Duration::from_millis(ms);
                let via_desc = evaluate_descriptor(&compiled.descriptor, &compiled.keyframes, now);
                let via_track = eval_track(track, now);
                assert_same_sample(&via_desc, &via_track);
                assert_eq!(via_desc.applied_value(), via_track.applied_value());
            }
        }
    }

    #[test]
    fn transform_descriptor_matches_evaluate_track() {
        let registry = MotionCodecRegistry::builtin();
        let from = PaintTransform {
            a: 1.0,
            b: 0.0,
            c: 0.0,
            d: 1.0,
            e: 0.0,
            f: 0.0,
        };
        let to = PaintTransform {
            a: 1.0,
            b: 0.0,
            c: 0.0,
            d: 1.0,
            e: 40.0,
            f: 8.0,
        };
        let track = transform_track(from, to);
        let compiled = compile_motion_descriptor(&track, &registry, None).expect("compile");
        assert_eq!(compiled.descriptor.codec, MotionCodecId::TRANSFORM);
        let now = Duration::from_millis(50);
        let via_desc = evaluate_descriptor(&compiled.descriptor, &compiled.keyframes, now);
        let via_track = eval_track(&track, now);
        assert_same_sample(&via_desc, &via_track);
        match via_desc.applied_value() {
            Some(MotionValue::Transform(value)) => {
                assert!((value.e - 20.0).abs() < 1e-5);
                assert!((value.f - 4.0).abs() < 1e-5);
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn width_does_not_compile_as_scale() {
        let registry = MotionCodecRegistry::builtin();
        let track = MotionTrack::transition(
            id(1),
            target(1),
            AnimatableProperty::Width,
            MotionValue::Scalar(10.0),
            MotionValue::Scalar(20.0),
            MotionTiming::new(
                Duration::ZERO,
                Duration::from_millis(100),
                Duration::from_millis(16),
            ),
            MotionCurve::Easing(Easing::Linear),
            AnimationPlayback::default(),
        );
        assert_eq!(track.execution_class(), AnimationClass::Layout);
        assert_eq!(
            compile_motion_descriptor(&track, &registry, None).unwrap_err(),
            MotionDescriptorError::NotCompositor
        );
    }

    #[test]
    fn paint_progress_and_unregistered_shader_do_not_compile() {
        let mut registry = MotionCodecRegistry::builtin();
        let progress = MotionTrack::transition(
            id(1),
            target(1),
            AnimatableProperty::Progress,
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
        assert_eq!(
            compile_motion_descriptor(&progress, &registry, None).unwrap_err(),
            MotionDescriptorError::NotCompositor
        );
        let mut shader = progress.clone();
        shader.property = AnimatableProperty::ShaderParameter;
        assert_eq!(
            compile_motion_descriptor(&shader, &registry, None).unwrap_err(),
            MotionDescriptorError::UnregisteredCodec
        );
        let codec = registry
            .register("nana.shader.glow", MotionValueKind::Scalar)
            .expect("register");
        compile_motion_descriptor(&shader, &registry, Some(codec)).expect("typed shader codec");
    }

    #[test]
    fn generational_handle_rejects_stale_after_free_and_reuse() {
        let mut store = MotionDescriptorStore::new();
        let track = opacity_track(1, 0.0, 1.0, MotionCurve::Easing(Easing::Linear));
        let first = store.bind(&track).expect("bind");
        let mid = store
            .evaluate(first, Duration::from_millis(50))
            .expect("live");
        match mid.value {
            MotionValue::Scalar(v) => assert!((v - 0.5).abs() < 1e-5),
            other => panic!("{other:?}"),
        }
        assert!(store.cancel(track.id));
        assert!(store.evaluate(first, Duration::from_millis(50)).is_none());
        assert!(store.get(first).is_none());

        let mut other = opacity_track(2, 0.2, 0.8, MotionCurve::Easing(Easing::Linear));
        other.from = MotionValue::Scalar(0.2);
        other.to = MotionTo::Value(MotionValue::Scalar(0.8));
        let second = store.bind(&other).expect("reuse");
        assert_eq!(second.index(), first.index());
        assert_ne!(second.generation(), first.generation());
        assert!(store.evaluate(first, Duration::from_millis(50)).is_none());
        let reused = store
            .evaluate(second, Duration::from_millis(50))
            .expect("new occupant");
        match reused.value {
            MotionValue::Scalar(v) => assert!((v - 0.5).abs() < 1e-5),
            other => panic!("{other:?}"),
        }
        match reused.applied_value() {
            Some(MotionValue::Scalar(v)) => assert!((v - 0.5).abs() < 1e-5),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn retarget_and_cancel_update_descriptor_in_place_or_free() {
        let mut store = MotionDescriptorStore::new();
        let track = opacity_track(1, 0.0, 1.0, MotionCurve::Easing(Easing::Linear));
        let handle = store.bind(&track).expect("bind");
        let generation = handle.generation();
        let epoch = store.structure_epoch();

        let mut retargeted = track.clone();
        retargeted.from = MotionValue::Scalar(0.5);
        retargeted.to = MotionTo::Value(MotionValue::Scalar(0.0));
        retargeted.timing.start = Duration::from_millis(50);
        let again = store.bind(&retargeted).expect("retarget");
        assert_eq!(again, handle);
        assert_eq!(again.generation(), generation);
        assert_ne!(store.structure_epoch(), epoch);
        let descriptor = store.get(handle).expect("updated");
        match (descriptor.from, descriptor.to) {
            (MotionValue::Scalar(from), MotionValue::Scalar(to)) => {
                assert!((from - 0.5).abs() < 1e-5);
                assert!((to - 0.0).abs() < 1e-5);
            }
            other => panic!("{other:?}"),
        }
        assert_eq!(descriptor.timing.start, Duration::from_millis(50));

        assert!(store.cancel(track.id));
        assert!(store.get(handle).is_none());
        assert!(store.evaluate(handle, Duration::from_millis(50)).is_none());
        assert!(store.is_empty());
    }

    #[test]
    fn steady_timestamps_do_not_rebuild_the_slab() {
        let mut store = MotionDescriptorStore::new();
        let track = opacity_track(1, 0.0, 1.0, MotionCurve::Easing(Easing::Linear));
        let handle = store.bind(&track).expect("bind");
        let epoch = store.structure_epoch();
        let capacity = store.slot_capacity();
        let live = store.live_len();
        for ms in 0..=100 {
            let sample = store
                .evaluate(handle, Duration::from_millis(ms))
                .expect("sample");
            assert_eq!(sample.applied_value().is_some(), sample.applies);
        }
        assert_eq!(store.structure_epoch(), epoch);
        assert_eq!(store.slot_capacity(), capacity);
        assert_eq!(store.live_len(), live);
    }

    #[test]
    fn fill_none_applies_matches_track() {
        let mut store = MotionDescriptorStore::new();
        let mut track = opacity_track(1, 0.0, 1.0, MotionCurve::Easing(Easing::Linear));
        track.playback.fill_mode = AnimationFillMode::None;
        track.timing.start = Duration::from_millis(50);
        let handle = store.bind(&track).expect("bind");
        let delay = store
            .evaluate(handle, Duration::from_millis(10))
            .expect("delay");
        assert!(!delay.applies);
        assert_eq!(delay.applied_value(), None);
        let mid = store
            .evaluate(handle, Duration::from_millis(100))
            .expect("mid");
        assert!(mid.applies);
        assert_eq!(
            mid.applied_value(),
            eval_track(&track, Duration::from_millis(100)).applied_value()
        );
    }

    #[test]
    fn keyframes_round_trip_through_the_slab() {
        let mut store = MotionDescriptorStore::new();
        let mut track = opacity_track(1, 0.0, 1.0, MotionCurve::Easing(Easing::Linear));
        track.to = MotionTo::Keyframes(vec![
            Keyframe {
                offset: 0.0,
                value: MotionValue::Scalar(0.0),
                easing: None,
            },
            Keyframe {
                offset: 1.0,
                value: MotionValue::Scalar(1.0),
                easing: Some(Easing::EaseOutCubic),
            },
        ]);
        let handle = store.bind(&track).expect("bind");
        let now = Duration::from_millis(50);
        assert_same_sample(
            &store.evaluate(handle, now).expect("sample"),
            &eval_track(&track, now),
        );
    }

    /// Consumers cache the packed tables on `(source, structure_epoch)`. Every
    /// store starts its epoch at 0, and a clone goes on counting its own from
    /// wherever it was copied, so a source shared with the original would let
    /// one slab be served for the other.
    #[test]
    fn a_clone_is_a_store_of_its_own() {
        let mut store = MotionDescriptorStore::new();
        assert_ne!(store.source(), MotionDescriptorStore::new().source());

        let mut clone = store.clone();
        assert_ne!(clone.source(), store.source());
        assert_eq!(clone.structure_epoch(), store.structure_epoch());

        // Diverging leaves the epochs equal while the tables differ, which is
        // exactly the case the sources have to separate.
        let curve = MotionCurve::Easing(Easing::Linear);
        store
            .bind(&opacity_track(1, 0.0, 1.0, curve))
            .expect("bind");
        clone
            .bind(&opacity_track(1, 0.0, 0.5, curve))
            .expect("bind");
        assert_eq!(clone.structure_epoch(), store.structure_epoch());
        assert_ne!(clone.pack_gpu().0, store.pack_gpu().0);
    }

    #[test]
    fn pack_gpu_indexes_match_handles_and_vacant_slots_keep_generation() {
        let mut store = MotionDescriptorStore::new();
        let track = opacity_track(1, 0.0, 1.0, MotionCurve::Easing(Easing::Linear));
        let handle = store.bind(&track).expect("bind");
        let (descriptors, _) = store.pack_gpu();
        assert_eq!(descriptors.len(), store.slot_capacity().max(1));
        let packed = descriptors[handle.index() as usize];
        assert!(packed.is_live());
        assert_eq!(packed.generation, handle.generation());
        assert_eq!(packed.property, 1);
        assert!(store.cancel(track.id));
        let (after, _) = store.pack_gpu();
        let vacated = after[handle.index() as usize];
        assert!(!vacated.is_live());
        assert_ne!(vacated.generation, handle.generation());
    }
}
