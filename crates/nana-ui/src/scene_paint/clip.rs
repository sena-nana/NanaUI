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

pub(super) const IDENTITY_AFFINE: [f32; 6] = [1.0, 0.0, 0.0, 1.0, 0.0, 0.0];

/// CSS/Canvas `matrix(a, b, c, d, e, f)`: `x' = ax + cy + e`, `y' = bx + dy + f`.
pub(super) fn transform_point([a, b, c, d, e, f]: [f32; 6], x: f32, y: f32) -> [f32; 2] {
    [a * x + c * y + e, b * x + d * y + f]
}

/// Scene origin is a post-translation of every primitive and clip.
pub(super) fn paint_affine(transform: [f32; 6], origin: [f32; 2]) -> [f32; 6] {
    let [a, b, c, d, e, f] = transform;
    [a, b, c, d, e + origin[0], f + origin[1]]
}

pub(super) fn is_translation([a, b, c, d, _, _]: [f32; 6]) -> bool {
    a == 1.0 && b == 0.0 && c == 0.0 && d == 1.0
}

fn near_zero(value: f32) -> bool {
    value.abs() <= 1e-6
}

/// True when a transformed rect stays axis-aligned (scale, translation, 90°).
/// GPU scissor matches the painted parallelogram in that case.
pub(super) fn is_axis_aligned([a, b, c, d, _, _]: [f32; 6]) -> bool {
    (near_zero(b) && near_zero(c)) || (near_zero(a) && near_zero(d))
}

/// Inverse of CSS/Canvas `matrix(a, b, c, d, e, f)`.
pub(super) fn invert_affine([a, b, c, d, e, f]: [f32; 6]) -> Option<[f32; 6]> {
    let det = a * d - b * c;
    if !det.is_finite() || det.abs() < 1e-12 {
        return None;
    }
    let inv_det = 1.0 / det;
    let ia = d * inv_det;
    let ib = -b * inv_det;
    let ic = -c * inv_det;
    let id = a * inv_det;
    Some([ia, ib, ic, id, -(ia * e + ic * f), -(ib * e + id * f)])
}

/// Inverse-affine point-in-rect for the innermost non-axis-aligned clip.
/// Axis-aligned clips stay on the GPU scissor (exact). Nested extra rotated
/// clips are not represented; shader clip uses this one parallelogram.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) struct FragmentClip {
    pub rect: [f32; 4],
    pub inv_abcd: [f32; 4],
    pub inv_ef: [f32; 2],
}

impl FragmentClip {
    /// Identity inverse + a rect that contains any paint-space coordinate.
    pub const PASS: Self = Self {
        rect: [-1.0e7, -1.0e7, 2.0e7, 2.0e7],
        inv_abcd: [1.0, 0.0, 0.0, 1.0],
        inv_ef: [0.0, 0.0],
    };

    /// Degenerate clip: no fragment is inside.
    pub const REJECT: Self = Self {
        rect: [0.0, 0.0, -1.0, -1.0],
        inv_abcd: [1.0, 0.0, 0.0, 1.0],
        inv_ef: [0.0, 0.0],
    };

    fn from_local(bounds: SceneRect, inverse: [f32; 6]) -> Self {
        Self {
            rect: [bounds.x, bounds.y, bounds.width, bounds.height],
            inv_abcd: [inverse[0], inverse[1], inverse[2], inverse[3]],
            inv_ef: [inverse[4], inverse[5]],
        }
    }
}

pub(super) fn fragment_clip(clips: &[nana_ui_scene::ClipRegion], origin: [f32; 2]) -> FragmentClip {
    clips
        .iter()
        .rev()
        .find_map(|clip| {
            let affine = paint_affine(clip.transform.0, origin);
            if is_axis_aligned(affine) {
                return None;
            }
            Some(match invert_affine(affine) {
                Some(inverse) => FragmentClip::from_local(clip.bounds, inverse),
                None => FragmentClip::REJECT,
            })
        })
        .unwrap_or(FragmentClip::PASS)
}

pub(super) fn point_in_fragment_clip(x: f32, y: f32, clip: FragmentClip) -> bool {
    let [local_x, local_y] = transform_point(
        [
            clip.inv_abcd[0],
            clip.inv_abcd[1],
            clip.inv_abcd[2],
            clip.inv_abcd[3],
            clip.inv_ef[0],
            clip.inv_ef[1],
        ],
        x,
        y,
    );
    local_x >= clip.rect[0]
        && local_y >= clip.rect[1]
        && local_x <= clip.rect[0] + clip.rect[2]
        && local_y <= clip.rect[1] + clip.rect[3]
}

pub(super) fn local_rect(bounds: SceneRect) -> LogicalRect {
    LogicalRect {
        x: bounds.x,
        y: bounds.y,
        width: bounds.width,
        height: bounds.height,
    }
}

/// Axis-aligned bounds of a rectangle after the affine is applied.
pub(super) fn transformed_aabb(bounds: LogicalRect, transform: [f32; 6]) -> LogicalRect {
    let corners = [
        transform_point(transform, bounds.x, bounds.y),
        transform_point(transform, bounds.x + bounds.width, bounds.y),
        transform_point(transform, bounds.x, bounds.y + bounds.height),
        transform_point(transform, bounds.x + bounds.width, bounds.y + bounds.height),
    ];
    let mut min_x = f32::INFINITY;
    let mut min_y = f32::INFINITY;
    let mut max_x = f32::NEG_INFINITY;
    let mut max_y = f32::NEG_INFINITY;
    for [x, y] in corners {
        min_x = min_x.min(x);
        min_y = min_y.min(y);
        max_x = max_x.max(x);
        max_y = max_y.max(y);
    }
    LogicalRect {
        x: min_x,
        y: min_y,
        width: (max_x - min_x).max(0.0),
        height: (max_y - min_y).max(0.0),
    }
}

pub(super) fn translated_rect(
    bounds: SceneRect,
    transform: [f32; 6],
    origin: [f32; 2],
) -> LogicalRect {
    transformed_aabb(local_rect(bounds), paint_affine(transform, origin))
}

pub(super) fn paint_origin(target_origin: [f32; 2], scene_origin: [f32; 2]) -> [f32; 2] {
    [
        target_origin[0] - scene_origin[0],
        target_origin[1] - scene_origin[1],
    ]
}

/// Transformed AABB scissor for GPU `set_scissor_rect`.
///
/// Rotated/sheared clips still contribute their AABB here as a coarse reject.
/// Quad and Mesh fragment shaders clip to [`fragment_clip`]. Nested extra
/// rotated clips, text glyphs, HostTexture, and custom nodes stay AABB-only.
/// Rounded HostTexture clip is the sibling Quad SDF, not this intersection.
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
    fn rotated_rect_aabb_covers_transformed_corners() {
        let bounds = LogicalRect::from_xywh(10.0, 20.0, 8.0, 4.0);
        let identity = transformed_aabb(bounds, IDENTITY_AFFINE);
        assert_eq!(identity, bounds);

        // 90° around origin: (x, y) -> (-y, x)
        let rotated = transformed_aabb(bounds, [0.0, 1.0, -1.0, 0.0, 0.0, 0.0]);
        assert_eq!(rotated.x, -24.0);
        assert_eq!(rotated.y, 10.0);
        assert_eq!(rotated.width, 4.0);
        assert_eq!(rotated.height, 8.0);
    }

    #[test]
    fn paint_affine_adds_scene_origin_to_translation() {
        assert_eq!(
            paint_affine([1.0, 0.0, 0.0, 1.0, 4.0, 6.0], [-40.0, -20.0]),
            [1.0, 0.0, 0.0, 1.0, -36.0, -14.0]
        );
        assert_eq!(
            transform_point([0.0, 1.0, -1.0, 0.0, 5.0, 7.0], 3.0, 4.0),
            [1.0, 10.0]
        );
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

    #[test]
    fn invert_affine_roundtrips_and_rejects_singular() {
        let original = [2.0, 0.5, -0.25, 3.0, 4.0, -6.0];
        let inverse = invert_affine(original).unwrap();
        let [x, y] = transform_point(original, 7.0, 11.0);
        let [rx, ry] = transform_point(inverse, x, y);
        assert!((rx - 7.0).abs() < 1e-5 && (ry - 11.0).abs() < 1e-5);
        assert_eq!(invert_affine(IDENTITY_AFFINE), Some(IDENTITY_AFFINE));
        assert!(invert_affine([1.0, 0.0, 2.0, 0.0, 3.0, 4.0]).is_none());
    }

    #[test]
    fn axis_aligned_includes_scale_and_right_angles() {
        assert!(is_axis_aligned(IDENTITY_AFFINE));
        assert!(is_axis_aligned([2.0, 0.0, 0.0, 0.5, 3.0, 4.0]));
        assert!(is_axis_aligned([0.0, 1.0, -1.0, 0.0, 0.0, 0.0]));
        assert!(!is_axis_aligned([0.707, 0.707, -0.707, 0.707, 0.0, 0.0]));
        assert!(!is_axis_aligned([1.0, 0.0, 0.5, 1.0, 0.0, 0.0]));
    }

    #[test]
    fn fragment_clip_rejects_aabb_corner_outside_rotated_rect() {
        let viewport = LogicalRect::viewport([0.0, 0.0], [64.0, 64.0]);
        let k = std::f32::consts::FRAC_1_SQRT_2;
        let clips = [ClipRegion {
            bounds: SceneRect {
                x: 16.0,
                y: 16.0,
                width: 32.0,
                height: 32.0,
            },
            transform: AffineTransform(
                nana_ui_core::PaintTransform {
                    a: k,
                    b: k,
                    c: -k,
                    d: k,
                    e: 0.0,
                    f: 0.0,
                }
                .around_center(16.0, 16.0, 32.0, 32.0),
            ),
        }];
        let origin = paint_origin([0.0, 0.0], [0.0, 0.0]);
        let aabb = intersect_clips(viewport, &clips, origin).unwrap();
        let clip = fragment_clip(&clips, origin);
        assert!(point_in_fragment_clip(32.0, 32.0, clip));

        let corner_x = aabb.x + 1.0;
        let corner_y = aabb.y + 1.0;
        assert!(
            corner_x >= aabb.x
                && corner_y >= aabb.y
                && corner_x <= aabb.x + aabb.width
                && corner_y <= aabb.y + aabb.height,
            "probe must sit in the transformed AABB scissor"
        );
        assert!(
            !point_in_fragment_clip(corner_x, corner_y, clip),
            "AABB corner {corner_x},{corner_y} must stay outside the rotated rect"
        );
    }

    #[test]
    fn fragment_clip_passes_when_every_clip_is_axis_aligned() {
        let clips = [ClipRegion {
            bounds: SceneRect {
                x: 10.0,
                y: 10.0,
                width: 20.0,
                height: 20.0,
            },
            transform: AffineTransform::IDENTITY,
        }];
        assert_eq!(
            fragment_clip(&clips, paint_origin([0.0, 0.0], [0.0, 0.0])),
            FragmentClip::PASS
        );
    }
}
