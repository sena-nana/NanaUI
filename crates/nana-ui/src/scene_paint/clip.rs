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
    transform_point_projective([a, b, c, d, e, f], [0.0, 0.0], x, y)
}

pub(super) fn transform_point_projective(
    [a, b, c, d, e, f]: [f32; 6],
    [g, h]: [f32; 2],
    x: f32,
    y: f32,
) -> [f32; 2] {
    let xp = a * x + c * y + e;
    let yp = b * x + d * y + f;
    let w = g * x + h * y + 1.0;
    if !w.is_finite() || w.abs() < 1e-8 {
        return [xp, yp];
    }
    [xp / w, yp / w]
}

/// Scene origin is a post-translation of every primitive and clip.
/// For a homography, that is `a' = a + ox*g` (not merely `e += ox`).
pub(super) fn paint_affine(transform: [f32; 6], origin: [f32; 2]) -> [f32; 6] {
    paint_transform(transform, [0.0, 0.0], origin).0
}

pub(super) fn paint_transform(
    [a, b, c, d, e, f]: [f32; 6],
    [g, h]: [f32; 2],
    origin: [f32; 2],
) -> ([f32; 6], [f32; 2]) {
    let ox = origin[0];
    let oy = origin[1];
    (
        [
            a + ox * g,
            b + oy * g,
            c + ox * h,
            d + oy * h,
            e + ox,
            f + oy,
        ],
        [g, h],
    )
}

pub(super) fn is_translation(transform: [f32; 6]) -> bool {
    is_translation_projective(transform, [0.0, 0.0])
}

pub(super) fn is_translation_projective([a, b, c, d, _, _]: [f32; 6], [g, h]: [f32; 2]) -> bool {
    a == 1.0 && b == 0.0 && c == 0.0 && d == 1.0 && g.abs() <= 1e-8 && h.abs() <= 1e-8
}

/// Pixel origin of a span of `logical_extent` centered on `logical_center`.
///
/// Icons and vertically centered text share this so a 12px glyph and a 12px
/// line box snap to the same physical top.
pub(super) fn snap_centered_origin(
    logical_center: f32,
    logical_extent: f32,
    scale: f32,
) -> (f32, f32) {
    let px = (logical_extent * scale).round().max(1.0);
    let origin = (logical_center * scale - px * 0.5).round();
    (origin, px)
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

/// Innermost non-axis-aligned clip parallelogram; extras dest-composite.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) struct FragmentClip {
    pub rect: [f32; 4],
    pub inv_abcd: [f32; 4],
    pub inv_ef: [f32; 2],
    /// Uniform inset-round radius in logical px; zero keeps axis-aligned rect clip.
    pub corner_radius: f32,
    /// `clip-path: polygon(...)` vertex count (≤ 8) in [`Self::rect`] local space.
    pub polygon_count: u8,
    pub polygon: [[f32; 2]; 8],
}

impl FragmentClip {
    /// Identity inverse + a rect that contains any paint-space coordinate.
    pub const PASS: Self = Self {
        rect: [-1.0e7, -1.0e7, 2.0e7, 2.0e7],
        inv_abcd: [1.0, 0.0, 0.0, 1.0],
        inv_ef: [0.0, 0.0],
        corner_radius: 0.0,
        polygon_count: 0,
        polygon: [[0.0; 2]; 8],
    };

    /// Degenerate clip: no fragment is inside.
    pub const REJECT: Self = Self {
        rect: [0.0, 0.0, -1.0, -1.0],
        inv_abcd: [1.0, 0.0, 0.0, 1.0],
        inv_ef: [0.0, 0.0],
        corner_radius: 0.0,
        polygon_count: 0,
        polygon: [[0.0; 2]; 8],
    };

    /// Exact bit pattern of the clip, for use as a resource-cache key.
    pub(super) fn to_bits(self) -> [u32; 30] {
        let mut bits = [0u32; 30];
        for (slot, value) in bits.iter_mut().zip(
            self.rect
                .iter()
                .chain(self.inv_abcd.iter())
                .chain(self.inv_ef.iter())
                .chain(std::iter::once(&self.corner_radius))
                .chain(std::iter::once(&(self.polygon_count as f32))),
        ) {
            *slot = value.to_bits();
        }
        let mut index = 14;
        for point in self.polygon {
            for coord in point {
                bits[index] = coord.to_bits();
                index += 1;
            }
        }
        bits
    }

    fn from_local(
        bounds: SceneRect,
        inverse: [f32; 6],
        corner_radius: f32,
        polygon: Option<&[[f32; 2]]>,
    ) -> Self {
        let mut clip = Self {
            rect: [bounds.x, bounds.y, bounds.width, bounds.height],
            inv_abcd: [inverse[0], inverse[1], inverse[2], inverse[3]],
            inv_ef: [inverse[4], inverse[5]],
            corner_radius,
            polygon_count: 0,
            polygon: [[0.0; 2]; 8],
        };
        if let Some(points) = polygon.filter(|points| points.len() >= 3) {
            let count = points.len().min(8);
            clip.polygon_count = count as u8;
            for (slot, point) in clip.polygon.iter_mut().zip(points.iter().take(8)) {
                *slot = *point;
            }
        }
        clip
    }

    /// Dest-group / affine text / icons sample physical pixels. Scale
    /// rect/radius/polygon; only `inv_ef` scales (linear inverse is invariant).
    pub(super) fn for_physical_pixels(self, scale: f32) -> Self {
        let scale = if scale.is_finite() && scale > 0.0 {
            scale
        } else {
            1.0
        };
        let mut polygon = self.polygon;
        if self.polygon_count > 0 {
            for point in polygon.iter_mut().take(self.polygon_count as usize) {
                point[0] *= scale;
                point[1] *= scale;
            }
        }
        Self {
            rect: [
                self.rect[0] * scale,
                self.rect[1] * scale,
                self.rect[2] * scale,
                self.rect[3] * scale,
            ],
            inv_abcd: self.inv_abcd,
            inv_ef: [self.inv_ef[0] * scale, self.inv_ef[1] * scale],
            corner_radius: self.corner_radius * scale,
            polygon_count: self.polygon_count,
            polygon,
        }
    }
}

fn axis_aligned_rounded_fragment_clips(
    clips: &[nana_ui_scene::ClipRegion],
    origin: [f32; 2],
) -> Vec<FragmentClip> {
    clips
        .iter()
        .filter_map(|clip| {
            if clip.transform.is_projective() {
                return None;
            }
            let affine = paint_affine(clip.transform.0, origin);
            if !is_axis_aligned(affine) || clip.corner_radius <= 0.0 {
                return None;
            }
            let inverse = invert_affine(affine)?;
            Some(FragmentClip::from_local(
                clip.bounds,
                inverse,
                clip.corner_radius,
                None,
            ))
        })
        .collect()
}

pub(super) fn polygon_fragment_clips(
    clips: &[nana_ui_scene::ClipRegion],
    origin: [f32; 2],
) -> Vec<FragmentClip> {
    clips
        .iter()
        .filter_map(|clip| {
            let points = clip.polygon_clip.as_ref()?;
            if points.len() < 3 {
                return None;
            }
            if clip.transform.is_projective() {
                return None;
            }
            let affine = paint_affine(clip.transform.0, origin);
            let inverse = invert_affine(affine)?;
            let local: Vec<[f32; 2]> = points.iter().copied().collect();
            Some(FragmentClip::from_local(
                clip.bounds,
                inverse,
                clip.corner_radius,
                Some(&local),
            ))
        })
        .collect()
}

/// Outer-to-inner non-axis-aligned clips. Empty when every clip is axis-aligned
/// (GPU scissor is exact) or the list is empty.
pub(super) fn rotated_fragment_clips(
    clips: &[nana_ui_scene::ClipRegion],
    origin: [f32; 2],
) -> Vec<FragmentClip> {
    clips
        .iter()
        .filter_map(|clip| {
            if clip.transform.is_projective() {
                return None;
            }
            let affine = paint_affine(clip.transform.0, origin);
            if is_axis_aligned(affine) {
                return None;
            }
            Some(match invert_affine(affine) {
                Some(inverse) => FragmentClip::from_local(
                    clip.bounds,
                    inverse,
                    clip.corner_radius,
                    clip.polygon_clip.as_deref(),
                ),
                None => FragmentClip::REJECT,
            })
        })
        .collect()
}

/// Innermost rotated clip for Quad/Mesh/Text/HostTexture vertex attrs.
/// Extra outers are [`extra_fragment_clips`] and dest-composited.
pub(super) fn fragment_clip(clips: &[nana_ui_scene::ClipRegion], origin: [f32; 2]) -> FragmentClip {
    if let Some(rotated) = rotated_fragment_clips(clips, origin)
        .into_iter()
        .next_back()
    {
        return rotated;
    }
    if let Some(polygon) = polygon_fragment_clips(clips, origin)
        .into_iter()
        .next_back()
    {
        return polygon;
    }
    axis_aligned_rounded_fragment_clips(clips, origin)
        .into_iter()
        .next_back()
        .unwrap_or(FragmentClip::PASS)
}

/// Dest extras around Quad/Mesh/Text/HostTexture. Innermost rotated/rounded
/// stays in vertex attrs. Polygons dest-wrap for quads; mesh drops the inner
/// polygon ([`mesh_extra_fragment_clips`]). Unique by bit pattern.
pub(super) fn extra_fragment_clips(
    clips: &[nana_ui_scene::ClipRegion],
    origin: [f32; 2],
) -> Vec<FragmentClip> {
    let inner_bits = fragment_clip(clips, origin).to_bits();
    let mut extras = Vec::new();
    for clip in rotated_fragment_clips(clips, origin) {
        if clip.to_bits() != inner_bits {
            push_unique_clip(&mut extras, clip);
        }
    }
    for clip in axis_aligned_rounded_fragment_clips(clips, origin) {
        if clip.to_bits() != inner_bits {
            push_unique_clip(&mut extras, clip);
        }
    }
    for clip in polygon_fragment_clips(clips, origin) {
        push_unique_clip(&mut extras, clip);
    }
    extras
}

fn push_unique_clip(extras: &mut Vec<FragmentClip>, clip: FragmentClip) {
    let bits = clip.to_bits();
    if extras.iter().any(|existing| existing.to_bits() == bits) {
        return;
    }
    extras.push(clip);
}

/// Mesh GpuClip owns the innermost polygon; drop it from dest extras.
pub(super) fn mesh_extra_fragment_clips(
    clips: &[nana_ui_scene::ClipRegion],
    origin: [f32; 2],
) -> Vec<FragmentClip> {
    let mut extras = extra_fragment_clips(clips, origin);
    let inner = fragment_clip(clips, origin);
    if inner.polygon_count >= 3 {
        extras.retain(|clip| clip.to_bits() != inner.to_bits());
    }
    extras
}

#[cfg(test)]
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
    if local_x < clip.rect[0]
        || local_y < clip.rect[1]
        || local_x > clip.rect[0] + clip.rect[2]
        || local_y > clip.rect[1] + clip.rect[3]
    {
        return false;
    }
    if clip.polygon_count >= 3 {
        let rel_x = local_x - clip.rect[0];
        let rel_y = local_y - clip.rect[1];
        if !point_in_polygon(rel_x, rel_y, clip.polygon_count, &clip.polygon) {
            return false;
        }
    }
    if clip.corner_radius <= 0.0 {
        return true;
    }
    let rel_x = local_x - clip.rect[0];
    let rel_y = local_y - clip.rect[1];
    let half_w = clip.rect[2] * 0.5;
    let half_h = clip.rect[3] * 0.5;
    let center_x = rel_x - half_w;
    let center_y = rel_y - half_h;
    let radius = clip.corner_radius.min(half_w).min(half_h);
    let q_x = center_x.abs() - half_w + radius;
    let q_y = center_y.abs() - half_h + radius;
    let outside = q_x.max(0.0).hypot(q_y.max(0.0)) - radius;
    let inside = q_x.max(q_y).min(0.0);
    inside + outside <= 0.0
}

#[cfg(test)]
fn point_in_polygon(x: f32, y: f32, count: u8, polygon: &[[f32; 2]; 8]) -> bool {
    if count < 3 {
        return true;
    }
    let mut winding = 0i32;
    for i in 0..count {
        let j = (i + 1) % count;
        let [x0, y0] = polygon[i as usize];
        let [x1, y1] = polygon[j as usize];
        if y0 <= y {
            if y1 > y {
                let cross = (x1 - x0) * (y - y0) - (x - x0) * (y1 - y0);
                if cross > 0.0 {
                    winding += 1;
                }
            }
        } else if y1 <= y {
            let cross = (x1 - x0) * (y - y0) - (x - x0) * (y1 - y0);
            if cross < 0.0 {
                winding -= 1;
            }
        }
    }
    winding != 0
}

/// Intersection of every rotated parallelogram. Axis-aligned clips are not in
/// `clips` here; they stay on the GPU scissor.
#[cfg(test)]
pub(super) fn point_in_fragment_clips(x: f32, y: f32, clips: &[FragmentClip]) -> bool {
    clips
        .iter()
        .copied()
        .all(|clip| point_in_fragment_clip(x, y, clip))
}

pub(super) fn local_rect(bounds: SceneRect) -> LogicalRect {
    LogicalRect {
        x: bounds.x,
        y: bounds.y,
        width: bounds.width,
        height: bounds.height,
    }
}

/// Axis-aligned bounds of a rectangle after the (possibly projective) transform.
pub(super) fn transformed_aabb(bounds: LogicalRect, transform: [f32; 6]) -> LogicalRect {
    transformed_aabb_projective(bounds, transform, [0.0, 0.0])
}

pub(super) fn transformed_aabb_projective(
    bounds: LogicalRect,
    transform: [f32; 6],
    persp: [f32; 2],
) -> LogicalRect {
    let corners = [
        transform_point_projective(transform, persp, bounds.x, bounds.y),
        transform_point_projective(transform, persp, bounds.x + bounds.width, bounds.y),
        transform_point_projective(transform, persp, bounds.x, bounds.y + bounds.height),
        transform_point_projective(
            transform,
            persp,
            bounds.x + bounds.width,
            bounds.y + bounds.height,
        ),
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
    translated_rect_projective(bounds, transform, [0.0, 0.0], origin)
}

pub(super) fn translated_rect_projective(
    bounds: SceneRect,
    transform: [f32; 6],
    persp: [f32; 2],
    origin: [f32; 2],
) -> LogicalRect {
    let (matrix, persp) = paint_transform(transform, persp, origin);
    transformed_aabb_projective(local_rect(bounds), matrix, persp)
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
/// Quad, Mesh, affine text, and HostTexture overflow clip to [`fragment_clip`]
/// (innermost parallelogram). Extra outer rotated clips dest-composite through
/// [`extra_fragment_clips`]. Custom nodes with a non-PASS fragment clip wrap
/// the same dest path so renderers need not implement parallelogram clip.
/// Rounded HostTexture clip is the sibling Quad SDF, not this intersection.
pub(super) fn intersect_clips(
    viewport: LogicalRect,
    clips: &[nana_ui_scene::ClipRegion],
    origin: [f32; 2],
) -> Option<LogicalRect> {
    clips.iter().try_fold(viewport, |visible, clip| {
        visible.intersection(translated_rect_projective(
            clip.bounds,
            clip.transform.0,
            clip.transform.1,
            origin,
        ))
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
    fn snap_centered_origin_rounds_the_span_then_the_top() {
        let (origin, px) = snap_centered_origin(14.0, 12.0, 2.0);
        assert_eq!(px, 24.0);
        assert_eq!(origin, 16.0);
    }

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
            transform: AffineTransform::from_matrix([1.0, 0.0, 0.0, 1.0, 5.0, 8.0]),
            corner_radius: 0.0,
            polygon_clip: None,
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
            transform: AffineTransform::from_matrix([1.0, 0.0, 0.0, 1.0, 200.0, 0.0]),
            corner_radius: 0.0,
            polygon_clip: None,
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
                corner_radius: 0.0,
                polygon_clip: None,
            },
            ClipRegion {
                bounds: SceneRect {
                    x: 50.0,
                    y: 30.0,
                    width: 20.0,
                    height: 10.0,
                },
                transform: AffineTransform::IDENTITY,
                corner_radius: 0.0,
                polygon_clip: None,
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
        assert!(is_translation(IDENTITY_AFFINE));
        assert!(!is_translation([0.0, 1.0, -1.0, 0.0, 5.0, 7.0]));
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
    fn paint_transform_bakes_scene_origin_into_homography() {
        let matrix = [1.0, 0.0, 0.0, 1.0, 4.0, 6.0];
        let persp = [0.01, 0.0];
        let origin = [-40.0, -20.0];
        let (baked, out_persp) = paint_transform(matrix, persp, origin);
        assert_eq!(out_persp, persp);
        assert!((baked[0] - (1.0 + origin[0] * persp[0])).abs() < 1e-6);
        assert_eq!(baked[4], 4.0 + origin[0]);
        assert_eq!(baked[5], 6.0 + origin[1]);
        let p = transform_point_projective(matrix, persp, 10.0, 20.0);
        let q = transform_point_projective(baked, out_persp, 10.0, 20.0);
        assert!((q[0] - (p[0] + origin[0])).abs() < 1e-4);
        assert!((q[1] - (p[1] + origin[1])).abs() < 1e-4);
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
            transform: AffineTransform::from_matrix(
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
            corner_radius: 0.0,
            polygon_clip: None,
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
            corner_radius: 0.0,
            polygon_clip: None,
        }];
        assert_eq!(
            fragment_clip(&clips, paint_origin([0.0, 0.0], [0.0, 0.0])),
            FragmentClip::PASS
        );
    }

    #[test]
    fn fragment_clip_physical_pixels_match_logical_probe() {
        let k = std::f32::consts::FRAC_1_SQRT_2;
        let clips = [ClipRegion {
            bounds: SceneRect {
                x: 16.0,
                y: 16.0,
                width: 32.0,
                height: 32.0,
            },
            transform: AffineTransform::from_matrix(
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
            corner_radius: 0.0,
            polygon_clip: None,
        }];
        let origin = paint_origin([0.0, 0.0], [0.0, 0.0]);
        let clip = fragment_clip(&clips, origin);
        let physical = clip.for_physical_pixels(2.0);
        assert!(point_in_fragment_clip(32.0, 32.0, clip));
        assert!(point_in_fragment_clip(64.0, 64.0, physical));
        assert!(!point_in_fragment_clip(1.0, 1.0, clip));
        assert!(!point_in_fragment_clip(2.0, 2.0, physical));
    }

    #[test]
    fn axis_aligned_rect_clip_rejects_outside() {
        let clip = FragmentClip::from_local(
            SceneRect {
                x: 16.0,
                y: 16.0,
                width: 32.0,
                height: 32.0,
            },
            IDENTITY_AFFINE,
            0.0,
            None,
        );
        assert!(point_in_fragment_clip(32.0, 32.0, clip));
        assert!(
            !point_in_fragment_clip(8.0, 8.0, clip),
            "single AABB reject must drop points outside the rect"
        );
        assert!(!point_in_fragment_clip(56.0, 32.0, clip));
    }

    #[test]
    fn fragment_clip_uses_axis_aligned_polygon() {
        let clips = [ClipRegion {
            bounds: SceneRect {
                x: 0.0,
                y: 0.0,
                width: 64.0,
                height: 64.0,
            },
            transform: AffineTransform::IDENTITY,
            corner_radius: 0.0,
            polygon_clip: Some(vec![[0.0, 0.0], [64.0, 0.0], [32.0, 64.0]]),
        }];
        let clip = fragment_clip(&clips, paint_origin([0.0, 0.0], [0.0, 0.0]));
        assert!(
            clip.polygon_count >= 3,
            "ancestor polygon must reach FragmentClip, not PASS AABB"
        );
        assert!(point_in_fragment_clip(32.0, 24.0, clip));
        assert!(!point_in_fragment_clip(4.0, 60.0, clip));
        let origin = paint_origin([0.0, 0.0], [0.0, 0.0]);
        assert_eq!(
            extra_fragment_clips(&clips, origin).len(),
            1,
            "quads dest-wrap the ancestor polygon (vertex locations full)"
        );
        assert!(
            mesh_extra_fragment_clips(&clips, origin).is_empty(),
            "mesh GpuClip owns the innermost polygon; no dest pass"
        );
    }

    #[test]
    fn overflow_aabb_plus_polygon_stays_scissor_and_gpu_clip() {
        let origin = paint_origin([0.0, 0.0], [0.0, 0.0]);
        let clips = [
            ClipRegion::axis_aligned(
                SceneRect {
                    x: 20.0,
                    y: 20.0,
                    width: 24.0,
                    height: 24.0,
                },
                AffineTransform::IDENTITY,
            ),
            ClipRegion {
                bounds: SceneRect {
                    x: 0.0,
                    y: 0.0,
                    width: 64.0,
                    height: 64.0,
                },
                transform: AffineTransform::IDENTITY,
                corner_radius: 0.0,
                polygon_clip: Some(vec![[0.0, 0.0], [64.0, 0.0], [32.0, 64.0]]),
            },
        ];
        let inner = fragment_clip(&clips, origin);
        assert!(
            inner.polygon_count >= 3,
            "innermost polygon must stay in GpuClip"
        );
        assert_eq!(
            extra_fragment_clips(&clips, origin).len(),
            1,
            "quads dest-wrap only the polygon; overflow AABB is scissor"
        );
        assert!(
            mesh_extra_fragment_clips(&clips, origin).is_empty(),
            "mesh dest must stay empty: overflow is scissor, polygon is GpuClip"
        );
    }

    #[test]
    fn inset_round_plus_polygon_keeps_round_as_dest() {
        let origin = paint_origin([0.0, 0.0], [0.0, 0.0]);
        let clips = [
            ClipRegion {
                bounds: SceneRect {
                    x: 0.0,
                    y: 0.0,
                    width: 64.0,
                    height: 64.0,
                },
                transform: AffineTransform::IDENTITY,
                corner_radius: 16.0,
                polygon_clip: None,
            },
            ClipRegion {
                bounds: SceneRect {
                    x: 0.0,
                    y: 0.0,
                    width: 64.0,
                    height: 64.0,
                },
                transform: AffineTransform::IDENTITY,
                corner_radius: 0.0,
                polygon_clip: Some(vec![[0.0, 0.0], [64.0, 0.0], [32.0, 64.0]]),
            },
        ];
        let inner = fragment_clip(&clips, origin);
        assert!(
            inner.polygon_count >= 3,
            "polygon wins fragment_clip over axis-aligned inset(round)"
        );
        let extras = extra_fragment_clips(&clips, origin);
        assert_eq!(
            extras.len(),
            2,
            "quads dest-wrap displaced inset(round) and the polygon, got {extras:?}"
        );
        assert!(
            extras
                .iter()
                .any(|clip| clip.corner_radius > 0.0 && clip.polygon_count < 3),
            "displaced inset(round) must dest-composite, got {extras:?}"
        );
        let mesh = mesh_extra_fragment_clips(&clips, origin);
        assert_eq!(mesh.len(), 1, "mesh dest is only the displaced round");
        assert!(
            mesh[0].corner_radius > 0.0 && mesh[0].polygon_count < 3,
            "mesh GpuClip owns the polygon; dest keeps ancestor inset(round), got {mesh:?}"
        );
    }

    #[test]
    fn rotated_outer_polygon_is_not_dest_wrapped_twice() {
        let k = std::f32::consts::FRAC_1_SQRT_2;
        let origin = paint_origin([0.0, 0.0], [0.0, 0.0]);
        let rotated = AffineTransform::from_matrix(
            nana_ui_core::PaintTransform {
                a: k,
                b: k,
                c: -k,
                d: k,
                ..Default::default()
            }
            .around_center(0.0, 0.0, 64.0, 64.0),
        );
        let clips = [
            ClipRegion {
                bounds: SceneRect {
                    x: 0.0,
                    y: 0.0,
                    width: 64.0,
                    height: 64.0,
                },
                transform: rotated,
                corner_radius: 0.0,
                polygon_clip: Some(vec![[0.0, 0.0], [64.0, 0.0], [32.0, 64.0]]),
            },
            ClipRegion {
                bounds: SceneRect {
                    x: 16.0,
                    y: 16.0,
                    width: 32.0,
                    height: 32.0,
                },
                transform: rotated,
                corner_radius: 0.0,
                polygon_clip: None,
            },
        ];
        let extras = extra_fragment_clips(&clips, origin);
        let polygon_dests = extras.iter().filter(|clip| clip.polygon_count >= 3).count();
        assert_eq!(
            polygon_dests, 1,
            "rotated outer polygon must dest-wrap once, got {extras:?}"
        );
        let mesh = mesh_extra_fragment_clips(&clips, origin);
        assert_eq!(mesh.len(), 1);
        assert!(
            mesh[0].polygon_count >= 3,
            "innermost is rotated overflow; mesh dest is the outer polygon"
        );
    }

    #[test]
    fn polygon_fragment_clip_accepts_triangle_interior() {
        let clip = polygon_fragment_clips(
            &[ClipRegion {
                bounds: SceneRect {
                    x: 0.0,
                    y: 0.0,
                    width: 64.0,
                    height: 64.0,
                },
                transform: AffineTransform::IDENTITY,
                corner_radius: 0.0,
                polygon_clip: Some(vec![[0.0, 0.0], [64.0, 0.0], [32.0, 64.0]]),
            }],
            paint_origin([0.0, 0.0], [0.0, 0.0]),
        )
        .pop()
        .expect("triangle clip");
        assert!(point_in_fragment_clip(32.0, 24.0, clip));
        assert!(!point_in_fragment_clip(4.0, 60.0, clip));
    }

    #[test]
    fn rotated_inset_round_rejects_local_corner_inside_sharp() {
        let k = std::f32::consts::FRAC_1_SQRT_2;
        let clips = [ClipRegion {
            bounds: SceneRect {
                x: 0.0,
                y: 0.0,
                width: 64.0,
                height: 64.0,
            },
            transform: AffineTransform::from_matrix(
                nana_ui_core::PaintTransform {
                    a: k,
                    b: k,
                    c: -k,
                    d: k,
                    ..Default::default()
                }
                .around_center(0.0, 0.0, 64.0, 64.0),
            ),
            corner_radius: 24.0,
            polygon_clip: None,
        }];
        let origin = paint_origin([0.0, 0.0], [-16.0, -16.0]);
        let rounded = fragment_clip(&clips, origin);
        let mut sharp = rounded;
        sharp.corner_radius = 0.0;
        // Dest (48.5, 8.5) matches the GPU pixel center.
        let px = 48.5;
        let py = 8.5;
        assert!(
            point_in_fragment_clip(px, py, sharp),
            "probe must sit inside the sharp rotated rect; clip={sharp:?}"
        );
        assert!(
            !point_in_fragment_clip(px, py, rounded),
            "r=24 SDF must reject the local corner; radius=0 would keep it"
        );
    }

    #[test]
    fn polygon_fragment_clip_physical_pixels_match_logical() {
        let clip = polygon_fragment_clips(
            &[ClipRegion {
                bounds: SceneRect {
                    x: 0.0,
                    y: 0.0,
                    width: 64.0,
                    height: 64.0,
                },
                transform: AffineTransform::IDENTITY,
                corner_radius: 0.0,
                polygon_clip: Some(vec![[0.0, 0.0], [64.0, 0.0], [32.0, 64.0]]),
            }],
            paint_origin([0.0, 0.0], [0.0, 0.0]),
        )
        .pop()
        .expect("triangle clip");
        let physical = clip.for_physical_pixels(1.0);
        assert!(point_in_fragment_clip(32.0, 24.0, clip));
        assert!(point_in_fragment_clip(32.0, 24.0, physical));
        assert!(!point_in_fragment_clip(4.0, 60.0, physical));
        let physical_2x = clip.for_physical_pixels(2.0);
        assert!(
            point_in_fragment_clip(64.0, 48.0, physical_2x),
            "2× dest pixels must keep the triangle interior"
        );
        assert!(!point_in_fragment_clip(8.0, 120.0, physical_2x));
    }

    fn nested_rotated_45_clips() -> ([ClipRegion; 2], [f32; 2]) {
        let k = std::f32::consts::FRAC_1_SQRT_2;
        // Outer is the smaller parallelogram so a probe can sit inside the
        // overflowing inner diamond and still miss the outer one.
        let outer = ClipRegion {
            bounds: SceneRect {
                x: 16.0,
                y: 16.0,
                width: 32.0,
                height: 32.0,
            },
            transform: AffineTransform::from_matrix(
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
            corner_radius: 0.0,
            polygon_clip: None,
        };
        let inner = ClipRegion {
            bounds: SceneRect {
                x: 0.0,
                y: 0.0,
                width: 64.0,
                height: 64.0,
            },
            transform: AffineTransform::from_matrix(
                nana_ui_core::PaintTransform {
                    a: k,
                    b: k,
                    c: -k,
                    d: k,
                    e: 0.0,
                    f: 0.0,
                }
                .around_center(0.0, 0.0, 64.0, 64.0),
            ),
            corner_radius: 0.0,
            polygon_clip: None,
        };
        ([outer, inner], paint_origin([0.0, 0.0], [0.0, 0.0]))
    }

    #[test]
    fn fragment_clip_uses_innermost_rotated_when_nested() {
        let ([outer, inner], origin) = nested_rotated_45_clips();
        let nested = [outer.clone(), inner.clone()];
        assert_eq!(
            fragment_clip(&nested, origin),
            fragment_clip(std::slice::from_ref(&inner), origin),
            "Quad vertex attrs still carry only the innermost parallelogram"
        );
        assert_eq!(
            extra_fragment_clips(&nested, origin),
            vec![fragment_clip(std::slice::from_ref(&outer), origin)]
        );
        assert!(extra_fragment_clips(std::slice::from_ref(&inner), origin).is_empty());
        assert!(extra_fragment_clips(&[outer], origin).is_empty());

        let rotated = rotated_fragment_clips(&nested, origin);
        assert_eq!(rotated.len(), 2);
        let inner_clip = fragment_clip(&[inner], origin);
        let mut probe = None;
        for y in 0..64u32 {
            for x in 0..64u32 {
                let px = x as f32 + 0.5;
                let py = y as f32 + 0.5;
                if point_in_fragment_clip(px, py, inner_clip)
                    && !point_in_fragment_clips(px, py, &rotated)
                {
                    probe = Some((px, py));
                    break;
                }
            }
            if probe.is_some() {
                break;
            }
        }
        let (px, py) = probe.expect(
            "nested extra must reject a point inside the inner parallelogram but outside the outer",
        );
        assert!(
            point_in_fragment_clip(px, py, inner_clip),
            "innermost-only would keep ({px},{py})"
        );
        assert!(
            !point_in_fragment_clips(px, py, &rotated),
            "AND of both 45° clips must reject ({px},{py})"
        );
        assert!(point_in_fragment_clips(32.0, 32.0, &rotated));
    }
}
