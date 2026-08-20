use nana_ui_scene::SceneRect;

use crate::PhysicalRect;

#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) struct LogicalRect {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

impl LogicalRect {
    #[cfg_attr(not(test), allow(dead_code))]
    pub const fn from_xywh(x: f32, y: f32, width: f32, height: f32) -> Self {
        Self {
            x,
            y,
            width,
            height,
        }
    }

    pub fn viewport(origin: [f32; 2], logical_size: [f32; 2]) -> Self {
        Self {
            x: origin[0],
            y: origin[1],
            width: logical_size[0].max(0.0),
            height: logical_size[1].max(0.0),
        }
    }

    pub fn intersection(self, other: Self) -> Option<Self> {
        let x = self.x.max(other.x);
        let y = self.y.max(other.y);
        let right = (self.x + self.width).min(other.x + other.width);
        let bottom = (self.y + self.height).min(other.y + other.height);
        let width = right - x;
        let height = bottom - y;
        if width <= 0.0 || height <= 0.0 {
            None
        } else {
            Some(Self {
                x,
                y,
                width,
                height,
            })
        }
    }

    pub fn to_core(self) -> crate::LogicalRect {
        crate::LogicalRect::new(self.x, self.y, self.width, self.height)
    }
}

pub(super) fn translated_rect(
    bounds: SceneRect,
    transform: [f32; 6],
    origin: [f32; 2],
) -> LogicalRect {
    LogicalRect {
        x: origin[0] + bounds.x + transform[4],
        y: origin[1] + bounds.y + transform[5],
        width: bounds.width,
        height: bounds.height,
    }
}

pub(super) fn paint_origin(target_origin: [f32; 2], scene_origin: [f32; 2]) -> [f32; 2] {
    [
        target_origin[0] - scene_origin[0],
        target_origin[1] - scene_origin[1],
    ]
}

pub(super) fn intersect_clips(
    viewport: LogicalRect,
    clips: &[nana_ui_scene::ClipRegion],
    origin: [f32; 2],
) -> Option<LogicalRect> {
    clips.iter().try_fold(viewport, |visible, clip| {
        visible.intersection(translated_rect(clip.bounds, clip.transform.0, origin))
    })
}

pub(super) fn physical_scissor(
    clip: LogicalRect,
    scale_factor: f32,
    target: [u32; 2],
) -> Option<PhysicalRect> {
    let scale = if scale_factor.is_finite() && scale_factor > 0.0 {
        scale_factor
    } else {
        1.0
    };
    let left = (clip.x * scale).floor();
    let top = (clip.y * scale).floor();
    let right = ((clip.x + clip.width) * scale).ceil();
    let bottom = ((clip.y + clip.height) * scale).ceil();
    let x = left.max(0.0).min(target[0] as f32);
    let y = top.max(0.0).min(target[1] as f32);
    let max_x = right.max(0.0).min(target[0] as f32);
    let max_y = bottom.max(0.0).min(target[1] as f32);
    let width = (max_x - x).max(0.0);
    let height = (max_y - y).max(0.0);
    if width < 1.0 || height < 1.0 {
        None
    } else {
        Some(PhysicalRect {
            x: x as u32,
            y: y as u32,
            width: width as u32,
            height: height as u32,
        })
    }
}

pub(super) fn physical_bounds(
    bounds: LogicalRect,
    scale_factor: f32,
    clip: PhysicalRect,
) -> PhysicalRect {
    let Some(slot) = physical_scissor(bounds, scale_factor, [u32::MAX, u32::MAX]) else {
        return PhysicalRect {
            x: 0,
            y: 0,
            width: 0,
            height: 0,
        };
    };
    intersect_physical(slot, clip)
}

pub(super) fn intersect_physical(left: PhysicalRect, right: PhysicalRect) -> PhysicalRect {
    let x = left.x.max(right.x);
    let y = left.y.max(right.y);
    let right_edge = left
        .x
        .saturating_add(left.width)
        .min(right.x.saturating_add(right.width));
    let bottom_edge = left
        .y
        .saturating_add(left.height)
        .min(right.y.saturating_add(right.height));
    PhysicalRect {
        x,
        y,
        width: right_edge.saturating_sub(x),
        height: bottom_edge.saturating_sub(y),
    }
}

#[cfg(test)]
mod tests {
    use nana_ui_scene::{AffineTransform, ClipRegion, SceneRect};

    use super::*;

    #[test]
    fn paint_origin_is_target_minus_scene() {
        assert_eq!(paint_origin([0.0, 0.0], [40.0, 20.0]), [-40.0, -20.0]);
        assert_eq!(paint_origin([200.0, 10.0], [40.0, 20.0]), [160.0, -10.0]);
        assert_eq!(paint_origin([200.0, 10.0], [0.0, 0.0]), [200.0, 10.0]);
    }

    #[test]
    fn scene_origin_subtracts_layout_origin() {
        let origin = paint_origin([0.0, 0.0], [40.0, 20.0]);
        let rect = translated_rect(
            SceneRect {
                x: 52.0,
                y: 48.0,
                width: 80.0,
                height: 32.0,
            },
            AffineTransform::IDENTITY.0,
            origin,
        );
        assert_eq!(rect.x, 12.0);
        assert_eq!(rect.y, 28.0);
        assert_eq!(rect.width, 80.0);
        assert_eq!(rect.height, 32.0);
    }

    #[test]
    fn clip_translation_intersects_viewport() {
        let viewport = LogicalRect::viewport([0.0, 0.0], [100.0, 80.0]);
        let clips = [ClipRegion {
            bounds: SceneRect {
                x: 10.0,
                y: 10.0,
                width: 50.0,
                height: 50.0,
            },
            transform: AffineTransform([1.0, 0.0, 0.0, 1.0, 5.0, 8.0]),
        }];
        let visible =
            intersect_clips(viewport, &clips, paint_origin([0.0, 0.0], [0.0, 0.0])).unwrap();
        assert_eq!(visible.x, 15.0);
        assert_eq!(visible.y, 18.0);
        assert_eq!(visible.width, 50.0);
        assert_eq!(visible.height, 50.0);
    }

    #[test]
    fn translated_clip_outside_viewport_is_empty() {
        let viewport = LogicalRect::viewport([0.0, 0.0], [100.0, 80.0]);
        let clips = [ClipRegion {
            bounds: SceneRect {
                x: 0.0,
                y: 0.0,
                width: 20.0,
                height: 20.0,
            },
            transform: AffineTransform([1.0, 0.0, 0.0, 1.0, 200.0, 0.0]),
        }];
        assert!(intersect_clips(viewport, &clips, paint_origin([0.0, 0.0], [0.0, 0.0])).is_none());
    }

    #[test]
    fn ancestor_clips_intersect_and_honor_scene_origin() {
        let viewport = LogicalRect::viewport([0.0, 0.0], [160.0, 80.0]);
        let clips = [
            ClipRegion {
                bounds: SceneRect {
                    x: 40.0,
                    y: 20.0,
                    width: 80.0,
                    height: 40.0,
                },
                transform: AffineTransform::IDENTITY,
            },
            ClipRegion {
                bounds: SceneRect {
                    x: 50.0,
                    y: 30.0,
                    width: 20.0,
                    height: 10.0,
                },
                transform: AffineTransform::IDENTITY,
            },
        ];
        let visible =
            intersect_clips(viewport, &clips, paint_origin([0.0, 0.0], [40.0, 20.0])).unwrap();
        assert_eq!(visible.x, 10.0);
        assert_eq!(visible.y, 10.0);
        assert_eq!(visible.width, 20.0);
        assert_eq!(visible.height, 10.0);
    }

    #[test]
    fn physical_scissor_is_in_target_pixels() {
        let clip = LogicalRect::from_xywh(10.25, 20.5, 100.5, 50.25);
        let scissor = physical_scissor(clip, 1.5, [400, 300]).unwrap();
        assert_eq!(scissor.x, 15);
        assert_eq!(scissor.y, 30);
        assert_eq!(scissor.width, 152);
        assert_eq!(scissor.height, 77);
    }
}
