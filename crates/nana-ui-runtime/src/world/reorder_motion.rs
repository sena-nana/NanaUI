//! A reorder list's live drag on the presentation layer: the carried rows
//! follow the pointer and the rows between make room where they would land.
//! Layout, hit testing of the drop and the rows' logical style stay as they
//! are; only compositor tracks move, and they all go when the drag ends.
use super::*;
use crate::component_animation_kinds::{REORDER_PREVIEW, REORDER_SHIFT};
use crate::reorder_list::ReorderDragVisual;
use nana_ui_core::PaintTransform;

/// Z-index for reorder preview layers
const REORDER_PREVIEW_Z_INDEX: i32 = 100;

/// The drag of a list visual, while it previews.
fn preview_drag(visual: Option<&StandardVisual>) -> Option<&ReorderDragVisual> {
    match visual {
        Some(StandardVisual::ReorderList {
            drag: Some(drag), ..
        }) if drag.preview => Some(drag),
        _ => None,
    }
}

/// Whether going from `previous` to `next` changes what a list paints. In a
/// previewed drag the list draws no drag marks, so a pointer move — which
/// only moves its rows' presentation — leaves the list itself as it was.
pub(super) fn reorder_list_paints_same(
    previous: Option<&StandardVisual>,
    next: Option<&StandardVisual>,
) -> bool {
    let (
        Some(StandardVisual::ReorderList {
            rows: previous_rows,
            size: previous_size,
            spacing: previous_spacing,
            ..
        }),
        Some(StandardVisual::ReorderList {
            rows,
            size,
            spacing,
            ..
        }),
        Some(previous_drag),
        Some(drag),
    ) = (previous, next, preview_drag(previous), preview_drag(next))
    else {
        return false;
    };
    previous_rows == rows
        && previous_size == size
        && previous_spacing == spacing
        && previous_drag.source == drag.source
}

impl UiWorld {
    /// The `z-index` a carried row of a previewed drag stacks at.
    pub(super) fn reorder_preview_z_index(&self, id: StableNodeId) -> Option<i32> {
        self.reorder_previews
            .contains_key(&id)
            .then_some(REORDER_PREVIEW_Z_INDEX)
    }

    /// Start, move or end list `id`'s drag tracks as its visual goes from
    /// `previous` to `next`. Called before the new visual is stored.
    pub(super) fn sync_reorder_list_motion(
        &mut self,
        id: StableNodeId,
        previous: Option<&StandardVisual>,
        next: Option<&StandardVisual>,
    ) {
        let had_preview = preview_drag(previous).is_some();
        let drag = preview_drag(next).cloned();
        if !had_preview && drag.is_none() {
            return;
        }
        let rows = self
            .nodes
            .get(id)
            .map(|node| node.hierarchy.children.to_vec())
            .unwrap_or_default();
        let boxes: Option<Vec<LayoutBox>> = rows
            .iter()
            .map(|row| self.component_layout_box(*row))
            .collect();
        // Rows that do not match the items yet (still being attached, or
        // not laid out) cannot be moved against their drop: no preview.
        let drag = drag.zip(boxes).filter(|(drag, boxes)| {
            boxes.len() == drag.rows() && *drag.carried().end() < boxes.len()
        });
        let carried = drag.as_ref().map_or(0..=0, |(drag, _)| drag.carried());
        let is_carried = |index: usize| drag.is_some() && carried.contains(&index);
        self.sync_reorder_previews(id, &rows, is_carried);
        let Some((drag, boxes)) = drag else {
            for row in rows {
                self.cancel_component_track(row, REORDER_PREVIEW);
                self.cancel_component_track(row, REORDER_SHIFT);
            }
            return;
        };
        let [dx, dy] = drag.preview_offset();
        let shifts = drag.shift_offsets(&boxes);
        let duration = if self.reduced_motion {
            Duration::ZERO
        } else {
            self.theme.duration(nana_ui_core::MotionRole::OverlayFade)
        };
        let easing = self
            .theme
            .motion()
            .easing(nana_ui_core::EasingRole::Standard);
        // A track holds its end value until the drag ends and cancels it.
        let hold = crate::AnimationPlayback::running(
            crate::AnimationIteration::ONCE,
            crate::AnimationDirection::Normal,
            crate::AnimationFillMode::Forwards,
        );
        for (index, &row) in rows.iter().enumerate() {
            let logical = self.logical_row_transform(row);
            if carried.contains(&index) {
                self.cancel_component_track(row, REORDER_SHIFT);
                let value = crate::MotionValue::Transform(translated(logical, dx, dy));
                // Direct manipulation, not motion: it follows the pointer
                // under reduced motion too. From and to are equal, so the
                // curve never shows.
                self.start_component_track(
                    row,
                    REORDER_PREVIEW,
                    Duration::from_nanos(1),
                    easing,
                    crate::AnimatableProperty::Transform,
                    value,
                    value,
                    crate::MotionInterrupt::Replace,
                    Some(hold),
                );
                continue;
            }
            self.cancel_component_track(row, REORDER_PREVIEW);
            let shift = shifts[index];
            if self.reorder_shift_target(row, logical) == shift {
                continue;
            }
            self.start_component_track(
                row,
                REORDER_SHIFT,
                duration,
                easing,
                crate::AnimatableProperty::Transform,
                crate::MotionValue::Transform(logical),
                crate::MotionValue::Transform(translated(logical, 0.0, shift)),
                crate::MotionInterrupt::Retarget,
                Some(hold),
            );
        }
    }

    /// Record which of `rows` of a list stack as its preview, and re-extract
    /// the rows whose stacking changes: their subtrees take the level with
    /// them, for paint and hit testing alike. Once when a drag starts and
    /// once when it ends; pointer moves change nothing here.
    fn sync_reorder_previews(
        &mut self,
        list: StableNodeId,
        rows: &[StableNodeId],
        is_carried: impl Fn(usize) -> bool,
    ) {
        let mut changed = Vec::new();
        self.reorder_previews.retain(|row, owner| {
            let keep = *owner != list
                || rows
                    .iter()
                    .position(|candidate| candidate == row)
                    .is_some_and(&is_carried);
            if !keep {
                changed.push(*row);
            }
            keep
        });
        for (index, &row) in rows.iter().enumerate() {
            if is_carried(index) && self.reorder_previews.insert(row, list).is_none() {
                changed.push(row);
            }
        }
        for row in changed {
            if self.nodes.contains(row) {
                self.mark_subtree(row, DirtyMask::RENDER | DirtyMask::INPUT);
            }
        }
    }

    /// Where a row's shift track is heading, as a vertical offset from its
    /// logical transform: 0 without one.
    fn reorder_shift_target(&self, row: StableNodeId, logical: PaintTransform) -> f32 {
        crate::component_animation_id(REORDER_SHIFT, row)
            .and_then(|id| self.presentation.get(id.track_id()))
            .and_then(|overlay| match overlay.track.to {
                crate::MotionTo::Value(crate::MotionValue::Transform(to)) => Some(to.f - logical.f),
                _ => None,
            })
            .unwrap_or(0.0)
    }

    fn logical_row_transform(&self, row: StableNodeId) -> PaintTransform {
        match self.logical_motion_value(row, crate::AnimatableProperty::Transform) {
            Some(crate::MotionValue::Transform(transform)) => transform,
            _ => PaintTransform::default(),
        }
    }

    fn cancel_component_track(&mut self, target: StableNodeId, kind: u64) {
        if let Some(id) = crate::component_animation_id(kind, target) {
            self.cancel_animation(id);
        }
    }

    /// A removed node no longer stacks as a preview, and a removed list's
    /// rows no longer do either.
    pub(super) fn forget_reorder_preview(&mut self, id: StableNodeId) {
        if !self.reorder_previews.is_empty() {
            self.reorder_previews
                .retain(|row, list| *row != id && *list != id);
        }
    }
}

/// `transform`, moved by `(dx, dy)` in the space it lands in.
fn translated(transform: PaintTransform, dx: f32, dy: f32) -> PaintTransform {
    PaintTransform {
        e: transform.e + dx,
        f: transform.f + dy,
        ..transform
    }
}
