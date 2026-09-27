//! Host-to-Nana coordinate conversion with revision-bound inverse caching.
//!
//! The bridge stores only host-provided presentation metadata. Layout and hit
//! geometry remain owned by `UiWorld`; this module does not build a parallel
//! transform tree.

use nana_diagnostics::metric;
use std::time::Instant;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InputCoordinateSpace {
    /// Application logical coordinates, before the host presentation transform.
    Logical,
    /// Host display logical coordinates, after the presentation transform.
    HostLogical,
    /// Host display physical pixels, converted using the host's two extents.
    Physical,
    /// Application surface UV, with top-left origin; independent of texture resolution.
    Normalized,
    /// Remote application surface UV, not normalized desktop coordinates.
    RemoteNormalized,
    /// Application surface UV returned by the host's ray/surface intersection.
    XrSurfaceUv,
    /// A nested Nana surface's parent-local coordinate, mapped by the host's
    /// authoritative embedding transform.
    ParentLocal,
    /// Viewport coordinates before content/scroll mapping.
    Viewport,
    /// Content coordinates after viewport/scroll mapping.
    Content,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CoordinateExtent {
    pub width: f32,
    pub height: f32,
}

impl CoordinateExtent {
    pub const fn new(width: f32, height: f32) -> Self {
        Self { width, height }
    }

    fn valid(self) -> bool {
        self.width.is_finite() && self.height.is_finite() && self.width > 0.0 && self.height > 0.0
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PresentationTransform {
    /// Affine logical → host logical transform `[a, b, c, d, e, f]`.
    pub affine: [f32; 6],
    pub revision: u64,
}

/// Host-provided application-space transform. The affine maps the named
/// source space directly into Nana logical coordinates; the host remains the
/// authority for the nested layout/viewport chain.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ApplicationSpaceTransform {
    pub affine: [f32; 6],
    pub revision: u64,
}

/// One host presentation snapshot. Extents and the transform revision are
/// applied together so a resize or dynamic-resolution update cannot expose a
/// partially updated coordinate contract to an input event.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PresentationCoordinateMetadata {
    pub logical_extent: CoordinateExtent,
    pub presentation_extent: CoordinateExtent,
    pub host_logical_extent: CoordinateExtent,
    pub host_physical_extent: CoordinateExtent,
    pub transform: PresentationTransform,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CoordinateError {
    InvalidExtent,
    NonInvertibleTransform,
    NonFiniteTransform,
    TransformRevisionConflict,
    NonFinitePoint,
    OutsideClip,
    MissingApplicationSpaceTransform(InputCoordinateSpace),
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct CoordinateBridgeCounters {
    pub map_queries: u64,
    pub inverse_recomputes: u64,
    pub clip_rejections: u64,
    pub mapping_latency_ns_total: u64,
}

#[derive(Debug, Clone, Copy)]
struct InverseCache {
    revision: u64,
    affine: Result<[f32; 6], CoordinateError>,
}

#[derive(Debug, Clone)]
pub struct PresentationCoordinateBridge {
    logical_extent: CoordinateExtent,
    presentation_extent: CoordinateExtent,
    host_logical_extent: CoordinateExtent,
    host_physical_extent: CoordinateExtent,
    transform: PresentationTransform,
    application_transforms: [Option<ApplicationSpaceTransform>; 3],
    inverse: Option<InverseCache>,
    counters: CoordinateBridgeCounters,
}

impl PresentationCoordinateBridge {
    pub fn new(
        logical_extent: CoordinateExtent,
        presentation_extent: CoordinateExtent,
        host_logical_extent: CoordinateExtent,
        host_physical_extent: CoordinateExtent,
        transform: PresentationTransform,
    ) -> Result<Self, CoordinateError> {
        let metadata = PresentationCoordinateMetadata {
            logical_extent,
            presentation_extent,
            host_logical_extent,
            host_physical_extent,
            transform,
        };
        Self::from_metadata(metadata)
    }

    pub fn from_metadata(
        metadata: PresentationCoordinateMetadata,
    ) -> Result<Self, CoordinateError> {
        validate_metadata(metadata)?;
        Ok(Self {
            logical_extent: metadata.logical_extent,
            presentation_extent: metadata.presentation_extent,
            host_logical_extent: metadata.host_logical_extent,
            host_physical_extent: metadata.host_physical_extent,
            transform: metadata.transform,
            application_transforms: [None, None, None],
            inverse: None,
            counters: CoordinateBridgeCounters::default(),
        })
    }

    /// Apply a complete host snapshot atomically. Older revisions and
    /// conflicting same-revision coefficients leave the previous snapshot
    /// untouched.
    pub fn set_metadata(
        &mut self,
        metadata: PresentationCoordinateMetadata,
    ) -> Result<(), CoordinateError> {
        validate_metadata(metadata)?;
        if metadata.transform.revision < self.transform.revision
            || (metadata.transform.revision == self.transform.revision
                && metadata.transform != self.transform)
        {
            return Err(CoordinateError::TransformRevisionConflict);
        }
        if metadata.transform.revision != self.transform.revision {
            self.inverse = None;
        }
        self.logical_extent = metadata.logical_extent;
        self.presentation_extent = metadata.presentation_extent;
        self.host_logical_extent = metadata.host_logical_extent;
        self.host_physical_extent = metadata.host_physical_extent;
        self.transform = metadata.transform;
        Ok(())
    }

    /// Revisions must increase when coefficients change. Repeating an identical
    /// revision is a no-op; conflicting or older metadata is rejected atomically.
    pub fn set_transform(
        &mut self,
        transform: PresentationTransform,
    ) -> Result<(), CoordinateError> {
        if !transform.affine.iter().all(|value| value.is_finite()) {
            return Err(CoordinateError::NonFiniteTransform);
        }
        if transform.revision < self.transform.revision
            || (transform.revision == self.transform.revision && transform != self.transform)
        {
            return Err(CoordinateError::TransformRevisionConflict);
        }
        if self.transform.revision != transform.revision {
            self.inverse = None;
            self.transform = transform;
        }
        Ok(())
    }

    pub fn set_extents(
        &mut self,
        logical_extent: CoordinateExtent,
        presentation_extent: CoordinateExtent,
        host_logical_extent: CoordinateExtent,
        host_physical_extent: CoordinateExtent,
    ) -> Result<(), CoordinateError> {
        if !logical_extent.valid()
            || !presentation_extent.valid()
            || !host_logical_extent.valid()
            || !host_physical_extent.valid()
        {
            return Err(CoordinateError::InvalidExtent);
        }
        self.logical_extent = logical_extent;
        self.presentation_extent = presentation_extent;
        self.host_logical_extent = host_logical_extent;
        self.host_physical_extent = host_physical_extent;
        Ok(())
    }

    pub fn counters(&self) -> CoordinateBridgeCounters {
        self.counters
    }

    /// Bind a host-owned nested coordinate mapping. The bridge stores the
    /// snapshot and revision only; it never derives a parallel geometry tree.
    pub fn set_application_space_transform(
        &mut self,
        space: InputCoordinateSpace,
        transform: ApplicationSpaceTransform,
    ) -> Result<(), CoordinateError> {
        let Some(slot) = application_transform_slot(space) else {
            return Err(CoordinateError::MissingApplicationSpaceTransform(space));
        };
        if !transform.affine.iter().all(|value| value.is_finite()) {
            return Err(CoordinateError::NonFiniteTransform);
        }
        let current = &mut self.application_transforms[slot];
        if current.is_some_and(|current| {
            transform.revision < current.revision
                || (transform.revision == current.revision && transform != current)
        }) {
            return Err(CoordinateError::TransformRevisionConflict);
        }
        *current = Some(transform);
        Ok(())
    }

    /// The authoritative host presentation revision currently bound to this
    /// bridge. Consumers can include it in input diagnostics without copying
    /// transform coefficients or forcing inverse recomputation.
    pub const fn transform_revision(&self) -> u64 {
        self.transform.revision
    }

    pub fn map(
        &mut self,
        space: InputCoordinateSpace,
        point: [f32; 2],
    ) -> Result<[f32; 2], CoordinateError> {
        let started = Instant::now();
        let result = self
            .map_unclipped(space, point)
            .and_then(|mapped| self.clip(mapped));
        let elapsed = started.elapsed().as_nanos().min(u128::from(u64::MAX)) as u64;
        self.counters.mapping_latency_ns_total = self
            .counters
            .mapping_latency_ns_total
            .saturating_add(elapsed);
        metric!(
            nana_diagnostics::framework::runtime::INPUT_MAPPING_NS,
            started.elapsed()
        );
        result
    }

    /// Map without rejecting points outside the application surface. A router
    /// uses this for an already captured pointer; node clips remain Runtime-owned.
    pub fn map_unclipped(
        &mut self,
        space: InputCoordinateSpace,
        point: [f32; 2],
    ) -> Result<[f32; 2], CoordinateError> {
        self.counters.map_queries += 1;
        if !point[0].is_finite() || !point[1].is_finite() {
            return Err(CoordinateError::NonFinitePoint);
        }
        let host_logical = match space {
            InputCoordinateSpace::Logical => return Ok(point),
            InputCoordinateSpace::HostLogical => point,
            InputCoordinateSpace::Physical => [
                (f64::from(point[0]) * f64::from(self.host_logical_extent.width)
                    / f64::from(self.host_physical_extent.width)) as f32,
                (f64::from(point[1]) * f64::from(self.host_logical_extent.height)
                    / f64::from(self.host_physical_extent.height)) as f32,
            ],
            InputCoordinateSpace::Normalized
            | InputCoordinateSpace::RemoteNormalized
            | InputCoordinateSpace::XrSurfaceUv => {
                return finite_point([
                    point[0] * self.logical_extent.width,
                    point[1] * self.logical_extent.height,
                ]);
            }
            space @ (InputCoordinateSpace::ParentLocal
            | InputCoordinateSpace::Viewport
            | InputCoordinateSpace::Content) => {
                let Some(slot) = application_transform_slot(space) else {
                    return Err(CoordinateError::MissingApplicationSpaceTransform(space));
                };
                let Some(transform) = self.application_transforms[slot] else {
                    return Err(CoordinateError::MissingApplicationSpaceTransform(space));
                };
                return finite_point(apply(transform.affine, point));
            }
        };
        let inverse = self.inverse_transform()?;
        finite_point(apply(inverse, host_logical))
    }

    fn inverse_transform(&mut self) -> Result<[f32; 6], CoordinateError> {
        if let Some(cache) = self.inverse
            && cache.revision == self.transform.revision
        {
            return cache.affine;
        }
        let inverse = invert(self.transform.affine).ok_or(CoordinateError::NonInvertibleTransform);
        self.inverse = Some(InverseCache {
            revision: self.transform.revision,
            affine: inverse,
        });
        self.counters.inverse_recomputes += 1;
        metric!(nana_diagnostics::framework::runtime::INPUT_INVERSE_RECOMPUTES);
        inverse
    }

    fn clip(&mut self, point: [f32; 2]) -> Result<[f32; 2], CoordinateError> {
        if point[0] < 0.0
            || point[1] < 0.0
            || point[0] > self.logical_extent.width
            || point[1] > self.logical_extent.height
        {
            self.counters.clip_rejections += 1;
            metric!(nana_diagnostics::framework::runtime::INPUT_CLIP_REJECTIONS);
            return Err(CoordinateError::OutsideClip);
        }
        Ok(point)
    }
}

fn application_transform_slot(space: InputCoordinateSpace) -> Option<usize> {
    match space {
        InputCoordinateSpace::ParentLocal => Some(0),
        InputCoordinateSpace::Viewport => Some(1),
        InputCoordinateSpace::Content => Some(2),
        _ => None,
    }
}

fn validate_metadata(metadata: PresentationCoordinateMetadata) -> Result<(), CoordinateError> {
    if !metadata.logical_extent.valid()
        || !metadata.presentation_extent.valid()
        || !metadata.host_logical_extent.valid()
        || !metadata.host_physical_extent.valid()
    {
        return Err(CoordinateError::InvalidExtent);
    }
    if !metadata
        .transform
        .affine
        .iter()
        .all(|value| value.is_finite())
    {
        return Err(CoordinateError::NonFiniteTransform);
    }
    Ok(())
}

fn apply(affine: [f32; 6], point: [f32; 2]) -> [f32; 2] {
    [
        affine[0] * point[0] + affine[2] * point[1] + affine[4],
        affine[1] * point[0] + affine[3] * point[1] + affine[5],
    ]
}

fn finite_point(point: [f32; 2]) -> Result<[f32; 2], CoordinateError> {
    if point.iter().all(|value| value.is_finite()) {
        Ok(point)
    } else {
        Err(CoordinateError::NonFinitePoint)
    }
}

fn invert(affine: [f32; 6]) -> Option<[f32; 6]> {
    // Intermediate f64 arithmetic avoids overflow and rejects only singular
    // matrices, rather than rejecting valid small scales using an epsilon.
    let affine = affine.map(f64::from);
    let det = affine[0] * affine[3] - affine[1] * affine[2];
    if !det.is_finite() || det == 0.0 {
        return None;
    }
    let inv = 1.0 / det;
    let inverse = [
        affine[3] * inv,
        -affine[1] * inv,
        -affine[2] * inv,
        affine[0] * inv,
        (affine[2] * affine[5] - affine[3] * affine[4]) * inv,
        (affine[1] * affine[4] - affine[0] * affine[5]) * inv,
    ]
    .map(|value| value as f32);
    inverse
        .iter()
        .all(|value| value.is_finite())
        .then_some(inverse)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bridge() -> PresentationCoordinateBridge {
        PresentationCoordinateBridge::new(
            CoordinateExtent::new(100.0, 100.0),
            CoordinateExtent::new(200.0, 200.0),
            CoordinateExtent::new(200.0, 200.0),
            CoordinateExtent::new(400.0, 400.0),
            PresentationTransform {
                affine: [2.0, 0.0, 0.0, 2.0, 10.0, 20.0],
                revision: 1,
            },
        )
        .unwrap()
    }

    #[test]
    fn host_mapping_uses_cached_inverse_until_revision_changes() {
        let mut bridge = bridge();
        assert_eq!(
            bridge.map(InputCoordinateSpace::HostLogical, [30.0, 40.0]),
            Ok([10.0, 10.0])
        );
        assert_eq!(
            bridge.map(InputCoordinateSpace::HostLogical, [30.0, 40.0]),
            Ok([10.0, 10.0])
        );
        assert_eq!(bridge.counters().inverse_recomputes, 1);
        bridge
            .set_transform(PresentationTransform {
                revision: 2,
                ..bridge.transform
            })
            .unwrap();
        bridge
            .map(InputCoordinateSpace::HostLogical, [30.0, 40.0])
            .unwrap();
        assert_eq!(bridge.counters().inverse_recomputes, 2);
    }

    #[test]
    fn normalized_mapping_is_logical_resolution_independent() {
        let mut bridge = bridge();
        assert_eq!(
            bridge.map(InputCoordinateSpace::Normalized, [0.1, 0.1]),
            Ok([10.0, 10.0])
        );
        bridge
            .set_extents(
                CoordinateExtent::new(100.0, 100.0),
                CoordinateExtent::new(800.0, 800.0),
                CoordinateExtent::new(400.0, 400.0),
                CoordinateExtent::new(800.0, 800.0),
            )
            .unwrap();
        assert_eq!(
            bridge.map(InputCoordinateSpace::Normalized, [0.1, 0.1]),
            Ok([10.0, 10.0])
        );
    }

    #[test]
    fn singular_transform_and_clip_are_explicit_errors() {
        let mut bridge = bridge();
        bridge
            .set_transform(PresentationTransform {
                affine: [0.0; 6],
                revision: 2,
            })
            .unwrap();
        assert_eq!(
            bridge.map(InputCoordinateSpace::HostLogical, [1.0, 1.0]),
            Err(CoordinateError::NonInvertibleTransform)
        );
    }

    #[test]
    fn rejects_nonfinite_transform_and_conflicting_revision() {
        let mut bridge = bridge();
        assert_eq!(
            bridge.set_transform(PresentationTransform {
                affine: [f32::NAN; 6],
                revision: 2,
            }),
            Err(CoordinateError::NonFiniteTransform)
        );
        assert_eq!(
            bridge.set_transform(PresentationTransform {
                affine: [1.0, 0.0, 0.0, 1.0, 0.0, 0.0],
                revision: 1,
            }),
            Err(CoordinateError::TransformRevisionConflict)
        );
    }

    #[test]
    fn capture_mapping_can_bypass_surface_clip() {
        let mut bridge = bridge();
        assert_eq!(
            bridge.map(InputCoordinateSpace::Logical, [101.0, 10.0]),
            Err(CoordinateError::OutsideClip)
        );
        assert_eq!(
            bridge.map_unclipped(InputCoordinateSpace::Logical, [101.0, 10.0]),
            Ok([101.0, 10.0])
        );
    }

    #[test]
    fn physical_mapping_honors_dpi_without_resolution_jump() {
        let mut bridge = PresentationCoordinateBridge::new(
            CoordinateExtent::new(100.0, 100.0),
            CoordinateExtent::new(200.0, 200.0),
            CoordinateExtent::new(200.0, 200.0),
            CoordinateExtent::new(400.0, 400.0),
            PresentationTransform {
                affine: [1.0, 0.0, 0.0, 1.0, 0.0, 0.0],
                revision: 1,
            },
        )
        .unwrap();
        assert_eq!(
            bridge.map(InputCoordinateSpace::Physical, [40.0, 40.0]),
            Ok([20.0, 20.0])
        );
        bridge
            .set_extents(
                CoordinateExtent::new(100.0, 100.0),
                CoordinateExtent::new(800.0, 800.0),
                CoordinateExtent::new(400.0, 400.0),
                CoordinateExtent::new(800.0, 800.0),
            )
            .unwrap();
        assert_eq!(
            bridge.map(InputCoordinateSpace::Physical, [40.0, 40.0]),
            Ok([20.0, 20.0])
        );
    }

    #[test]
    fn high_frequency_host_mapping_reuses_revision_bound_inverse() {
        let mut bridge = bridge();
        for index in 0..1_000 {
            let point = [30.0 + (index as f32 % 3.0), 40.0];
            assert!(bridge.map(InputCoordinateSpace::HostLogical, point).is_ok());
        }
        let counters = bridge.counters();
        assert_eq!(counters.map_queries, 1_000);
        assert_eq!(counters.inverse_recomputes, 1);
        assert!(counters.mapping_latency_ns_total > 0);
    }

    #[test]
    fn rotated_nonuniform_presentation_transform_round_trips() {
        let mut bridge = PresentationCoordinateBridge::new(
            CoordinateExtent::new(100.0, 100.0),
            CoordinateExtent::new(100.0, 100.0),
            CoordinateExtent::new(200.0, 200.0),
            CoordinateExtent::new(400.0, 400.0),
            PresentationTransform {
                // logical -> host: 90 degree rotation with non-uniform scale
                // and translation.
                affine: [0.0, 2.0, -3.0, 0.0, 120.0, 40.0],
                revision: 7,
            },
        )
        .unwrap();
        let logical = [20.0, 30.0];
        let host = [30.0, 80.0];
        assert_eq!(
            bridge.map(InputCoordinateSpace::HostLogical, host),
            Ok(logical)
        );
        assert_eq!(bridge.transform_revision(), 7);
        assert_eq!(bridge.counters().inverse_recomputes, 1);
    }

    #[test]
    fn metadata_update_rejects_conflicts_without_partial_extent_write() {
        let mut bridge = bridge();
        let before = bridge
            .map(InputCoordinateSpace::HostLogical, [30.0, 40.0])
            .unwrap();
        let error = bridge.set_metadata(PresentationCoordinateMetadata {
            logical_extent: CoordinateExtent::new(200.0, 200.0),
            presentation_extent: CoordinateExtent::new(800.0, 800.0),
            host_logical_extent: CoordinateExtent::new(300.0, 300.0),
            host_physical_extent: CoordinateExtent::new(600.0, 600.0),
            transform: PresentationTransform {
                affine: [1.0; 6],
                revision: 1,
            },
        });
        assert_eq!(error, Err(CoordinateError::TransformRevisionConflict));
        assert_eq!(
            bridge.map(InputCoordinateSpace::HostLogical, [30.0, 40.0]),
            Ok(before)
        );
    }

    #[test]
    fn tiny_but_invertible_scale_is_accepted() {
        let mut bridge = PresentationCoordinateBridge::new(
            CoordinateExtent::new(1.0, 1.0),
            CoordinateExtent::new(1.0, 1.0),
            CoordinateExtent::new(1.0, 1.0),
            CoordinateExtent::new(1.0, 1.0),
            PresentationTransform {
                affine: [1.0e-10, 0.0, 0.0, 1.0e-10, 0.0, 0.0],
                revision: 1,
            },
        )
        .unwrap();
        assert_eq!(
            bridge.map(InputCoordinateSpace::HostLogical, [1.0e-10, 1.0e-10]),
            Ok([1.0, 1.0])
        );
    }

    #[test]
    fn nested_application_spaces_use_host_revisioned_mapping() {
        let mut bridge = bridge();
        assert_eq!(
            bridge.map(InputCoordinateSpace::ParentLocal, [1.0, 2.0]),
            Err(CoordinateError::MissingApplicationSpaceTransform(
                InputCoordinateSpace::ParentLocal
            ))
        );
        for (space, affine, point, expected) in [
            (
                InputCoordinateSpace::ParentLocal,
                [1.0, 0.0, 0.0, 1.0, 10.0, 20.0],
                [1.0, 2.0],
                [11.0, 22.0],
            ),
            (
                InputCoordinateSpace::Viewport,
                [2.0, 0.0, 0.0, 2.0, 0.0, 0.0],
                [3.0, 4.0],
                [6.0, 8.0],
            ),
            (
                InputCoordinateSpace::Content,
                [1.0, 0.0, 0.0, 1.0, -5.0, 7.0],
                [8.0, 2.0],
                [3.0, 9.0],
            ),
        ] {
            bridge
                .set_application_space_transform(
                    space,
                    ApplicationSpaceTransform {
                        affine,
                        revision: 1,
                    },
                )
                .unwrap();
            assert_eq!(bridge.map(space, point), Ok(expected));
        }
        assert_eq!(
            bridge.set_application_space_transform(
                InputCoordinateSpace::Viewport,
                ApplicationSpaceTransform {
                    affine: [1.0; 6],
                    revision: 0,
                },
            ),
            Err(CoordinateError::TransformRevisionConflict)
        );
    }
}
