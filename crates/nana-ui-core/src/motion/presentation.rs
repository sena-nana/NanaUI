//! Transient presentation overlay over Motion IR.
//!
//! Logical values stay in the retained tree. This store holds in-flight
//! [`crate::motion::MotionTrack`]s so compositor-safe properties can be queried
//! at an arbitrary timestamp without writing UiWorld each frame.
//!
//! ## Same-property composition
//!
//! For one `(target, property)`, the winning overlay is the applying track with
//! the latest [`crate::motion::MotionTiming::start`]. Equal starts: higher
//! [`crate::motion::MotionTrackId`] wins. Tracks with `applies = false` are
//! skipped. Values are never blended.

use std::{collections::HashMap, time::Duration};

use super::{
    PresentationPair,
    eval::evaluate_track,
    ir::{MotionSample, MotionTargetId, MotionTrack, MotionTrackId, MotionValue},
    property::AnimatableProperty,
    track_completion_deadline,
};

/// One overlay record. `logical` is the UiWorld target captured at
/// start/retarget; presentation is always `evaluate_track` at the query time.
#[derive(Debug, Clone, PartialEq)]
pub struct PresentationOverlay {
    pub track: MotionTrack,
    pub logical: MotionValue,
}

/// Overlay of Motion tracks in the same id space as Runtime `AnimationId`.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct PresentationStore {
    tracks: HashMap<MotionTrackId, PresentationOverlay>,
    by_property: HashMap<(MotionTargetId, AnimatableProperty), Vec<MotionTrackId>>,
}

impl PresentationStore {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn is_empty(&self) -> bool {
        self.tracks.is_empty()
    }

    pub fn len(&self) -> usize {
        self.tracks.len()
    }

    pub fn get(&self, id: MotionTrackId) -> Option<&PresentationOverlay> {
        self.tracks.get(&id)
    }

    pub fn get_mut(&mut self, id: MotionTrackId) -> Option<&mut PresentationOverlay> {
        self.tracks.get_mut(&id)
    }

    pub fn insert(&mut self, track: MotionTrack, logical: MotionValue) {
        let id = track.id;
        let key = (track.target, track.property);
        if let Some(previous) = self
            .tracks
            .insert(id, PresentationOverlay { track, logical })
        {
            let old_key = (previous.track.target, previous.track.property);
            if old_key != key {
                self.unindex(old_key, id);
                self.index(key, id);
            }
        } else {
            self.index(key, id);
        }
    }

    pub fn remove(&mut self, id: MotionTrackId) -> Option<PresentationOverlay> {
        let overlay = self.tracks.remove(&id)?;
        self.unindex((overlay.track.target, overlay.track.property), id);
        Some(overlay)
    }

    pub fn remove_target(&mut self, target: MotionTargetId) -> Vec<PresentationOverlay> {
        let ids = self
            .tracks
            .values()
            .filter(|overlay| overlay.track.target == target)
            .map(|overlay| overlay.track.id)
            .collect::<Vec<_>>();
        ids.into_iter().filter_map(|id| self.remove(id)).collect()
    }

    /// Winning applying sample for `(target, property)` at `now`.
    pub fn sample(
        &self,
        target: MotionTargetId,
        property: AnimatableProperty,
        now: Duration,
    ) -> Option<MotionSample> {
        let id = self.winner_id(target, property, now)?;
        let overlay = self.tracks.get(&id)?;
        Some(evaluate_track(&overlay.track, now))
    }

    pub fn applied_value(
        &self,
        target: MotionTargetId,
        property: AnimatableProperty,
        now: Duration,
    ) -> Option<MotionValue> {
        self.sample(target, property, now)
            .and_then(MotionSample::applied_value)
    }

    pub fn pair(
        &self,
        target: MotionTargetId,
        property: AnimatableProperty,
        logical: MotionValue,
        now: Duration,
    ) -> PresentationPair {
        PresentationPair {
            logical,
            presentation: self.applied_value(target, property, now).unwrap_or(logical),
        }
    }

    /// Unfinished overlay for `target` at `now`. Park/logical unmount must not
    /// drop these; despawn (identity gone) still removes them.
    pub fn retains_unmounted(&self, target: MotionTargetId, now: Duration) -> bool {
        self.tracks.values().any(|overlay| {
            overlay.track.target == target && !evaluate_track(&overlay.track, now).finished
        })
    }

    pub fn completion_deadline(&self, id: MotionTrackId) -> Option<Duration> {
        track_completion_deadline(&self.tracks.get(&id)?.track)
    }

    /// Overlay records. Scene compositor promotion reads this without
    /// copying evaluation.
    pub fn overlays(&self) -> impl Iterator<Item = &PresentationOverlay> {
        self.tracks.values()
    }

    /// Whether `(target, property)` has an overlay record. Does not evaluate.
    pub fn has_property(&self, target: MotionTargetId, property: AnimatableProperty) -> bool {
        self.by_property
            .get(&(target, property))
            .is_some_and(|ids| !ids.is_empty())
    }

    /// Whether any overlay record exists for `property`. Does not evaluate.
    pub fn has_any_property(&self, property: AnimatableProperty) -> bool {
        self.by_property.keys().any(|(_, prop)| *prop == property)
    }

    /// Winning applying track for `(target, property)` at `now`.
    pub fn winning_track_id(
        &self,
        target: MotionTargetId,
        property: AnimatableProperty,
        now: Duration,
    ) -> Option<MotionTrackId> {
        self.winner_id(target, property, now)
    }

    fn winner_id(
        &self,
        target: MotionTargetId,
        property: AnimatableProperty,
        now: Duration,
    ) -> Option<MotionTrackId> {
        let ids = self.by_property.get(&(target, property))?;
        let mut best: Option<(Duration, MotionTrackId)> = None;
        for id in ids {
            let Some(overlay) = self.tracks.get(id) else {
                continue;
            };
            let sample = evaluate_track(&overlay.track, now);
            if sample.applied_value().is_none() {
                continue;
            }
            let start = overlay.track.timing.start;
            let better = match best {
                None => true,
                Some((best_start, best_id)) => {
                    start > best_start || (start == best_start && *id > best_id)
                }
            };
            if better {
                best = Some((start, *id));
            }
        }
        best.map(|(_, id)| id)
    }

    fn index(&mut self, key: (MotionTargetId, AnimatableProperty), id: MotionTrackId) {
        let list = self.by_property.entry(key).or_default();
        if !list.contains(&id) {
            list.push(id);
        }
    }

    fn unindex(&mut self, key: (MotionTargetId, AnimatableProperty), id: MotionTrackId) {
        let Some(list) = self.by_property.get_mut(&key) else {
            return;
        };
        list.retain(|existing| *existing != id);
        if list.is_empty() {
            self.by_property.remove(&key);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::motion::{
        AnimationFillMode, AnimationIteration, AnimationPlayback, MotionCurve, MotionTiming,
        easing::Easing,
    };

    fn id(n: u64) -> MotionTrackId {
        MotionTrackId::new(n).unwrap()
    }

    fn target(n: u64) -> MotionTargetId {
        MotionTargetId::new(n).unwrap()
    }

    fn opacity_track(
        track_id: u64,
        start_ms: u64,
        duration_ms: u64,
        from: f32,
        to: f32,
    ) -> MotionTrack {
        MotionTrack::transition(
            id(track_id),
            target(1),
            AnimatableProperty::Opacity,
            MotionValue::Scalar(from),
            MotionValue::Scalar(to),
            MotionTiming::new(
                Duration::from_millis(start_ms),
                Duration::from_millis(duration_ms),
                Duration::from_millis(16),
            ),
            MotionCurve::Easing(Easing::Linear),
            AnimationPlayback::default(),
        )
    }

    #[test]
    fn later_start_wins_same_property_and_values_are_not_blended() {
        let mut store = PresentationStore::new();
        store.insert(opacity_track(1, 0, 100, 0.0, 1.0), MotionValue::Scalar(1.0));
        store.insert(
            opacity_track(2, 40, 100, 0.2, 0.8),
            MotionValue::Scalar(1.0),
        );
        let mid = store
            .applied_value(
                target(1),
                AnimatableProperty::Opacity,
                Duration::from_millis(50),
            )
            .expect("later track applies");
        match mid {
            MotionValue::Scalar(value) => {
                assert!((value - 0.26).abs() < 1e-5, "got {value}");
            }
            other => panic!("expected scalar, got {other:?}"),
        }
        let pair = store.pair(
            target(1),
            AnimatableProperty::Opacity,
            MotionValue::Scalar(1.0),
            Duration::from_millis(50),
        );
        assert_eq!(pair.logical, MotionValue::Scalar(1.0));
        assert_ne!(pair.logical, pair.presentation);
    }

    #[test]
    fn equal_start_higher_id_wins() {
        let mut store = PresentationStore::new();
        store.insert(opacity_track(1, 0, 100, 0.0, 1.0), MotionValue::Scalar(1.0));
        store.insert(opacity_track(3, 0, 100, 0.5, 0.5), MotionValue::Scalar(1.0));
        let value = store
            .applied_value(
                target(1),
                AnimatableProperty::Opacity,
                Duration::from_millis(50),
            )
            .expect("applies");
        assert_eq!(value, MotionValue::Scalar(0.5));
    }

    #[test]
    fn applies_false_is_skipped_and_applied_value_is_none() {
        let mut store = PresentationStore::new();
        let mut delayed = opacity_track(1, 50, 100, 0.0, 1.0);
        delayed.playback.fill_mode = AnimationFillMode::None;
        store.insert(delayed, MotionValue::Scalar(1.0));
        assert_eq!(
            store.applied_value(
                target(1),
                AnimatableProperty::Opacity,
                Duration::from_millis(10)
            ),
            None
        );
        let sample = evaluate_track(&store.get(id(1)).unwrap().track, Duration::from_millis(10));
        assert!(!sample.applies);
        assert_eq!(sample.value, MotionValue::Scalar(0.0));
        assert_eq!(sample.applied_value(), None);
    }

    #[test]
    fn fill_forwards_keeps_overlay_after_finish() {
        let mut store = PresentationStore::new();
        let mut track = opacity_track(1, 0, 100, 0.0, 1.0);
        track.playback = AnimationPlayback::running(
            AnimationIteration::ONCE,
            crate::motion::AnimationDirection::Normal,
            AnimationFillMode::Forwards,
        );
        store.insert(track, MotionValue::Scalar(1.0));
        let late = store
            .sample(
                target(1),
                AnimatableProperty::Opacity,
                Duration::from_millis(500),
            )
            .expect("fill forwards");
        assert!(late.finished);
        assert_eq!(late.applied_value(), Some(MotionValue::Scalar(1.0)));
    }

    #[test]
    fn unfinished_overlay_retains_unmounted_target() {
        let mut store = PresentationStore::new();
        store.insert(opacity_track(1, 0, 100, 0.0, 1.0), MotionValue::Scalar(1.0));
        assert!(store.retains_unmounted(target(1), Duration::from_millis(40)));
        assert!(!store.retains_unmounted(target(1), Duration::from_millis(100)));
    }
}
