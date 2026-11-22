//! Per-window swapchain backbuffer damage tracking for partial presentation.

use std::collections::{HashMap, VecDeque};

use crate::geometry::PhysicalRect;
use crate::gpu_view::{intersect_physical, union_physical};
use crate::{
    AlphaEncoding, ScenePaintViewport, ScenePresentationParameters, ScenePresentationProfile,
};

const DAMAGE_HISTORY_CAPACITY: usize = 32;

/// Scene/target configuration tuple that must match across consecutive frames
/// for incremental host-texture damage to be valid on a preserved backbuffer.
#[derive(Clone, PartialEq)]
pub(super) struct DamageSceneKey {
    pub instance_id: u64,
    pub presentation_epoch: u64,
    pub viewport: ScenePaintViewport,
    pub profile: ScenePresentationProfile,
    pub alpha: AlphaEncoding,
    pub text_aa: Option<crate::scene_paint::SubpixelOrder>,
    pub parameters: ScenePresentationParameters,
}

#[derive(Clone, PartialEq)]
pub(super) struct PlannedFrameDamage {
    pub configure_generation: u64,
    pub buffer_index: Option<u32>,
    pub scene_key: DamageSceneKey,
    pub current_frame_damage: Option<PhysicalRect>,
    pub backbuffer_damage: Option<PhysicalRect>,
    pub present_damage: Option<PhysicalRect>,
    pub requested_partial: bool,
}

#[derive(Default)]
pub(super) struct WindowDamageTracker {
    configure_generation: u64,
    last_scene_key: Option<DamageSceneKey>,
    pub(super) slot_revisions: HashMap<nana_ui_runtime::TextureSlotName, u64>,
    present_seq: u64,
    history: VecDeque<(u64, Option<PhysicalRect>)>,
    buffer_last_seq: HashMap<u32, u64>,
}

impl WindowDamageTracker {
    pub(super) fn reset(&mut self, configure_generation: u64) {
        self.configure_generation = configure_generation;
        self.last_scene_key = None;
        self.slot_revisions.clear();
        self.present_seq = 0;
        self.history.clear();
        self.buffer_last_seq.clear();
    }

    pub(super) fn begin_frame(
        &mut self,
        configure_generation: u64,
        buffer_index: Option<u32>,
        preserved: bool,
        scene_key: DamageSceneKey,
        current_damage: Option<PhysicalRect>,
    ) -> PlannedFrameDamage {
        if self.configure_generation != configure_generation {
            self.reset(configure_generation);
        }

        let viewport_rect = PhysicalRect {
            x: 0,
            y: 0,
            width: scene_key.viewport.physical_size[0],
            height: scene_key.viewport.physical_size[1],
        };
        let scene_matches = self.last_scene_key.as_ref() == Some(&scene_key);
        let current_frame_damage = if scene_matches {
            current_damage.map(|rect| intersect_physical(rect, viewport_rect))
        } else {
            None
        };

        let mut backbuffer_damage = None;
        if preserved
            && let Some(buf_idx) = buffer_index
            && let Some(cur_rect) = current_frame_damage
            && let Some(&prev_seq) = self.buffer_last_seq.get(&buf_idx)
            && let Some(history_rect) = self.union_since(prev_seq)
        {
            let combined =
                intersect_physical(union_physical(cur_rect, history_rect), viewport_rect);
            let total_px = u64::from(viewport_rect.width) * u64::from(viewport_rect.height);
            let dirty_px = u64::from(combined.width) * u64::from(combined.height);
            if combined.width > 0
                && combined.height > 0
                && total_px > 0
                && dirty_px.saturating_mul(4) < total_px.saturating_mul(3)
            {
                backbuffer_damage = Some(combined);
            }
        }

        PlannedFrameDamage {
            configure_generation,
            buffer_index,
            scene_key,
            current_frame_damage,
            backbuffer_damage,
            present_damage: backbuffer_damage,
            requested_partial: backbuffer_damage.is_some(),
        }
    }

    pub(super) fn commit_presented(
        &mut self,
        plan: PlannedFrameDamage,
        direct_presented: bool,
        used_partial: bool,
    ) {
        if self.configure_generation != plan.configure_generation {
            self.reset(plan.configure_generation);
        }
        self.present_seq = self.present_seq.saturating_add(1);
        let seq = self.present_seq;

        let recorded_delta = if direct_presented && (used_partial || !plan.requested_partial) {
            plan.current_frame_damage
        } else {
            None
        };
        if self.history.len() >= DAMAGE_HISTORY_CAPACITY {
            self.history.pop_front();
        }
        self.history.push_back((seq, recorded_delta));

        if let Some(buf_idx) = plan.buffer_index {
            self.buffer_last_seq.insert(buf_idx, seq);
        }
        if direct_presented {
            self.last_scene_key = Some(plan.scene_key);
        } else {
            self.last_scene_key = None;
        }
    }

    fn union_since(&self, after_seq: u64) -> Option<PhysicalRect> {
        if after_seq == self.present_seq {
            return Some(PhysicalRect {
                x: 0,
                y: 0,
                width: 0,
                height: 0,
            });
        }
        let earliest = self.history.front().map_or(0, |(seq, _)| *seq);
        if after_seq + 1 < earliest {
            return None;
        }
        let mut combined = PhysicalRect {
            x: 0,
            y: 0,
            width: 0,
            height: 0,
        };
        for &(seq, rect) in &self.history {
            if seq > after_seq {
                let rect = rect?;
                combined = union_physical(combined, rect);
            }
        }
        Some(combined)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_scene_key(width: u32, height: u32, epoch: u64) -> DamageSceneKey {
        DamageSceneKey {
            instance_id: 7,
            presentation_epoch: epoch,
            viewport: ScenePaintViewport {
                logical_size: [width as f32, height as f32],
                physical_size: [width, height],
                scale_factor: 1.0,
                scene_origin: [0.0, 0.0],
                target_origin: [0.0, 0.0],
                clear_color: [0.0, 0.0, 0.0, 1.0],
                clear: true,
            },
            profile: ScenePresentationProfile::new(
                nana_gpu::GpuTextureFormat::BGRA8_UNORM_SRGB,
                crate::ScenePresentationColorSpace::Srgb,
            ),
            alpha: AlphaEncoding::Linear,
            text_aa: None,
            parameters: ScenePresentationParameters::default(),
        }
    }

    fn rect(x: u32, y: u32, width: u32, height: u32) -> PhysicalRect {
        PhysicalRect {
            x,
            y,
            width,
            height,
        }
    }

    #[test]
    fn tracks_two_and_three_buffer_flip_rings() {
        let mut tracker = WindowDamageTracker::default();
        let key = test_scene_key(1000, 1000, 1);

        // Frame 1 -> buffer 0 (unpreserved initial frame)
        let p1 = tracker.begin_frame(1, Some(0), false, key.clone(), Some(rect(10, 10, 50, 50)));
        assert_eq!(p1.backbuffer_damage, None);
        tracker.commit_presented(p1, true, false);

        // Frame 2 -> buffer 1 (unpreserved initial frame, known delta r2)
        let r2 = rect(20, 20, 40, 40);
        let p2 = tracker.begin_frame(1, Some(1), false, key.clone(), Some(r2));
        assert_eq!(p2.backbuffer_damage, None);
        tracker.commit_presented(p2, true, false);

        // Frame 3 -> buffer 2 (unpreserved initial frame on triple buffer, delta r3)
        let r3 = rect(100, 100, 40, 40);
        let p3 = tracker.begin_frame(1, Some(2), false, key.clone(), Some(r3));
        assert_eq!(p3.backbuffer_damage, None);
        tracker.commit_presented(p3, true, false);

        // Frame 4 -> buffer 0 returns (preserved, needs r2 | r3 | r4)
        let r4 = rect(200, 50, 30, 30);
        let p4 = tracker.begin_frame(1, Some(0), true, key.clone(), Some(r4));
        assert_eq!(p4.backbuffer_damage, Some(rect(20, 20, 210, 120)));
        assert_eq!(p4.present_damage, p4.backbuffer_damage);
        tracker.commit_presented(p4, true, true);

        // Frame 5 -> buffer 1 returns (preserved, needs r3 | r4 | r5)
        let r5 = rect(300, 300, 20, 20);
        let p5 = tracker.begin_frame(1, Some(1), true, key, Some(r5));
        assert_eq!(p5.backbuffer_damage, Some(rect(100, 50, 220, 270)));
    }

    #[test]
    fn resets_on_configure_generation_and_scene_key_change() {
        let mut tracker = WindowDamageTracker::default();
        let key1 = test_scene_key(1000, 1000, 1);

        let p1 = tracker.begin_frame(1, Some(0), false, key1.clone(), Some(rect(10, 10, 20, 20)));
        tracker.commit_presented(p1, true, false);
        let p2 = tracker.begin_frame(1, Some(1), false, key1.clone(), Some(rect(30, 30, 20, 20)));
        tracker.commit_presented(p2, true, false);

        // Scene key epoch change invalidates partial damage for current frame and breaks ring for buf 0 later.
        let key2 = test_scene_key(1000, 1000, 2);
        let p3 = tracker.begin_frame(1, Some(0), true, key2.clone(), Some(rect(40, 40, 20, 20)));
        assert_eq!(p3.backbuffer_damage, None);
        assert_eq!(p3.current_frame_damage, None);
        tracker.commit_presented(p3, true, false);

        // Buffer 1 crosses seq 3 (None), so it must also do a full repaint once.
        let p4 = tracker.begin_frame(1, Some(1), true, key2.clone(), Some(rect(50, 50, 20, 20)));
        assert_eq!(p4.backbuffer_damage, None);
        tracker.commit_presented(p4, true, false);

        // Buffer 0 now crosses only seq 4 (Some(r4)) with matching key2 -> partial resumes!
        let p5 = tracker.begin_frame(1, Some(0), true, key2.clone(), Some(rect(60, 60, 20, 20)));
        assert_eq!(p5.backbuffer_damage, Some(rect(50, 50, 30, 30)));
        tracker.commit_presented(p5, true, true);

        // Resize bumps configure_generation -> full reset.
        let p6 = tracker.begin_frame(2, Some(0), true, key2, Some(rect(60, 60, 20, 20)));
        assert_eq!(p6.backbuffer_damage, None);
        assert_eq!(tracker.configure_generation, 2);
    }

    #[test]
    fn falls_back_above_area_threshold_and_on_history_overflow() {
        let mut tracker = WindowDamageTracker::default();
        let key = test_scene_key(1000, 1000, 1);

        let p1 = tracker.begin_frame(1, Some(0), false, key.clone(), None);
        tracker.commit_presented(p1, true, false);

        // Large damage >= 75% area falls back cleanly.
        let p2 = tracker.begin_frame(1, Some(0), true, key.clone(), Some(rect(0, 0, 900, 900)));
        assert_eq!(p2.backbuffer_damage, None);
        tracker.commit_presented(p2, true, false);

        // Push 35 frames on buffer 1 without touching buffer 0 -> history ring overflows for buffer 0.
        for i in 0..35 {
            let p = tracker.begin_frame(
                1,
                Some(1),
                i > 0,
                key.clone(),
                Some(rect(10 + i, 10, 20, 20)),
            );
            let used_partial = p.backbuffer_damage.is_some();
            tracker.commit_presented(p, true, used_partial);
        }

        let stale_buf0 = tracker.begin_frame(1, Some(0), true, key, Some(rect(10, 10, 20, 20)));
        assert_eq!(stale_buf0.backbuffer_damage, None);
    }
}
