//! Curve flattening, `i_overlay` booleans/outlines, `lyon` triangulation, and triangle clipping.

use i_overlay::core::fill_rule::FillRule as OverlayFill;
use i_overlay::core::overlay_rule::OverlayRule;
use i_overlay::float::simplify::SimplifyShape;
use i_overlay::float::single::SingleFloatOverlay;
use i_overlay::mesh::float::outline::offset::OutlineOffset;
use i_overlay::mesh::float::style::{LineJoin as OverlayJoin, OutlineStyle};
use lyon_tessellation::path::iterator::PathIterator;
use lyon_tessellation::path::{Path as LyonPath, PathEvent};
use lyon_tessellation::{
    BuffersBuilder, FillOptions, FillRule as LyonFill, FillTessellator, FillVertex, VertexBuffers,
    VertexSource,
};

use super::*;

// ---------------------------------------------------------------------------
// Geometry
// ---------------------------------------------------------------------------

pub(super) fn lyon_point(p: [f32; 2]) -> lyon_tessellation::math::Point {
    lyon_tessellation::math::point(p[0], p[1])
}

pub(super) fn lyon_path(path: &PaintPath, t: Affine) -> LyonPath {
    let lyon_point = |p: [f32; 2]| lyon_point(affine_apply(t, p));
    let mut builder = LyonPath::builder();
    let mut open = false;
    let mut cursor = [0.0f32; 2];
    let mut start = [0.0f32; 2];
    let ensure_open = |builder: &mut lyon_tessellation::path::path::Builder,
                       open: &mut bool,
                       cursor: [f32; 2]| {
        let lyon_point = |p: [f32; 2]| lyon_point(affine_apply(t, p));
        if !*open {
            builder.begin(lyon_point(cursor));
            *open = true;
        }
    };
    for verb in path.verbs() {
        match *verb {
            PathVerb::MoveTo(p) => {
                if open {
                    builder.end(false);
                }
                builder.begin(lyon_point(p));
                open = true;
                cursor = p;
                start = p;
            }
            PathVerb::LineTo(p) => {
                ensure_open(&mut builder, &mut open, cursor);
                builder.line_to(lyon_point(p));
                cursor = p;
            }
            PathVerb::QuadTo(c, p) => {
                ensure_open(&mut builder, &mut open, cursor);
                builder.quadratic_bezier_to(lyon_point(c), lyon_point(p));
                cursor = p;
            }
            PathVerb::CubicTo(a, b, p) => {
                ensure_open(&mut builder, &mut open, cursor);
                builder.cubic_bezier_to(lyon_point(a), lyon_point(b), lyon_point(p));
                cursor = p;
            }
            PathVerb::Close => {
                if open {
                    builder.end(true);
                    open = false;
                }
                cursor = start;
            }
        }
    }
    if open {
        builder.end(false);
    }
    builder.build()
}

/// Whether every point of `path` under `t` is finite and within
/// [`MAX_COORDINATE`]. A path with a non-finite point has no shape to draw,
/// and one far past any screen would flatten into millions of segments.
pub(super) fn path_is_drawable(path: &PaintPath, t: Affine) -> bool {
    let sane = |p: &[f32; 2]| {
        affine_apply(t, *p)
            .iter()
            .all(|v| v.is_finite() && v.abs() <= MAX_COORDINATE)
    };
    path.verbs().iter().all(|verb| match verb {
        PathVerb::MoveTo(p) | PathVerb::LineTo(p) => sane(p),
        PathVerb::QuadTo(c, p) => sane(c) && sane(p),
        PathVerb::CubicTo(a, b, p) => sane(a) && sane(b) && sane(p),
        PathVerb::Close => true,
    })
}

/// How far from the node's origin a path may reach, under its transform,
/// and still be drawn: a million pixels is far past any screen.
pub(super) const MAX_COORDINATE: f32 = nana_ui_runtime::MAX_PATH_COORDINATE;

/// The longest stroke width, blur or spread drawn as asked.
pub(super) const MAX_LENGTH: f32 = 1.0e4;

/// Sub-paths as polylines under `t`, each with whether it was closed.
/// `tolerance` is in the space `t` maps to.
pub(super) fn flatten(path: &PaintPath, t: Affine, tolerance: f32) -> Vec<(Contour, bool)> {
    if !path_is_drawable(path, t) {
        return Vec::new();
    }
    let mut out = Vec::new();
    let mut current: Contour = Vec::new();
    for event in lyon_path(path, t).iter().flattened(tolerance) {
        match event {
            PathEvent::Begin { at } => {
                current = vec![[f64::from(at.x), f64::from(at.y)]];
            }
            PathEvent::Line { to, .. } => {
                push_point(&mut current, [f64::from(to.x), f64::from(to.y)])
            }
            PathEvent::End { close, .. } => {
                let mut contour = std::mem::take(&mut current);
                if contour.len() > 1 && same(contour[0], contour[contour.len() - 1]) {
                    contour.pop();
                }
                out.push((contour, close));
            }
            PathEvent::Quadratic { .. } | PathEvent::Cubic { .. } => {}
        }
    }
    out
}

pub(super) fn push_point(contour: &mut Contour, point: Point) {
    if point[0].is_finite()
        && point[1].is_finite()
        && contour.last().is_none_or(|last| !same(*last, point))
    {
        contour.push(point);
    }
}

pub(super) fn same(a: Point, b: Point) -> bool {
    (a[0] - b[0]).abs() < 1e-4 && (a[1] - b[1]).abs() < 1e-4
}

pub(super) fn simplify_contours(contours: Vec<Contour>, rule: FillRule) -> Shapes {
    if contours.is_empty() {
        return Vec::new();
    }
    let rule = match rule {
        FillRule::NonZero => OverlayFill::NonZero,
        FillRule::EvenOdd => OverlayFill::EvenOdd,
    };
    contours.simplify_shape(rule)
}

pub(super) fn difference_shapes(outer: &Shapes, inner: &Shapes) -> Shapes {
    if inner.is_empty() {
        return outer.clone();
    }
    outer.overlay(inner, OverlayRule::Difference, OverlayFill::NonZero)
}

pub(super) fn outline_shapes(shapes: Shapes, offset: f32) -> Shapes {
    shapes.outline(&OutlineStyle::new(f64::from(offset)).line_join(OverlayJoin::Round(0.1)))
}

pub(super) fn intersect(subject: &Shapes, clip: &Shapes) -> Shapes {
    if subject.is_empty() || clip.is_empty() {
        return Vec::new();
    }
    subject.overlay(clip, OverlayRule::Intersect, OverlayFill::NonZero)
}

pub(super) fn rounded_shapes(rect: SceneRect, radii: [f32; 4], t: Affine) -> Shapes {
    let mut path = PaintPath::new();
    path.rounded_rect(
        LayoutBox {
            x: rect.x,
            y: rect.y,
            width: rect.width,
            height: rect.height,
        },
        radii,
    );
    fill_shapes(&path, t)
}

pub(super) fn triangulate(shapes: &Shapes) -> Vec<[[f32; 2]; 3]> {
    let Some(Tessellation {
        vertices, indices, ..
    }) = tessellate(shapes.iter().flatten())
    else {
        return Vec::new();
    };
    indices
        .as_chunks::<3>()
        .0
        .iter()
        .map(|tri| {
            let mut tri = [
                vertices[tri[0] as usize],
                vertices[tri[1] as usize],
                vertices[tri[2] as usize],
            ];
            if cross(tri[0], tri[1], tri[2]) < 0.0 {
                tri.swap(1, 2);
            }
            tri
        })
        .collect()
}

pub(super) struct Tessellation {
    pub(super) vertices: Vec<[f32; 2]>,
    pub(super) indices: Vec<u32>,
    /// Per vertex, the `[contour, point]` of the outline it is, or `None` for
    /// one the tessellator added where contours cross.
    pub(super) outline: Vec<Option<[usize; 2]>>,
}

/// Interior triangles of a set of closed contours, even-odd.
pub(super) fn tessellate<'a>(contours: impl Iterator<Item = &'a Contour>) -> Option<Tessellation> {
    let mut builder = LyonPath::builder();
    // Indexed by the endpoint lyon assigned each outline point.
    let mut endpoints = Vec::new();
    let mut record = |id: lyon_tessellation::path::EndpointId, at: [usize; 2]| {
        if endpoints.len() <= id.to_usize() {
            endpoints.resize(id.to_usize() + 1, None);
        }
        endpoints[id.to_usize()] = Some(at);
    };
    for (index, contour) in contours.enumerate() {
        let mut points = contour.iter();
        let Some(first) = points.next() else {
            continue;
        };
        if contour.len() < 3 {
            continue;
        }
        record(
            builder.begin(lyon_point([first[0] as f32, first[1] as f32])),
            [index, 0],
        );
        for (point, p) in points.enumerate() {
            record(
                builder.line_to(lyon_point([p[0] as f32, p[1] as f32])),
                [index, point + 1],
            );
        }
        builder.end(true);
    }
    if endpoints.is_empty() {
        return None;
    }
    let path = builder.build();
    let mut buffers: VertexBuffers<([f32; 2], Option<[usize; 2]>), u32> = VertexBuffers::new();
    FillTessellator::new()
        .tessellate_with_ids(
            path.id_iter(),
            &path,
            None,
            &FillOptions::tolerance(TOLERANCE).with_fill_rule(LyonFill::EvenOdd),
            &mut BuffersBuilder::new(&mut buffers, |vertex: FillVertex| {
                let outline = vertex.sources().find_map(|source| match source {
                    VertexSource::Endpoint { id } => endpoints.get(id.to_usize()).copied()?,
                    VertexSource::Edge { .. } => None,
                });
                (vertex.position().to_array(), outline)
            }),
        )
        .ok()?;
    if buffers.indices.is_empty() {
        return None;
    }
    let (vertices, outline) = buffers.vertices.into_iter().unzip();
    Some(Tessellation {
        vertices,
        indices: buffers.indices,
        outline,
    })
}

pub(super) fn cross(a: [f32; 2], b: [f32; 2], c: [f32; 2]) -> f32 {
    (b[0] - a[0]) * (c[1] - a[1]) - (b[1] - a[1]) * (c[0] - a[0])
}

pub(super) fn signed_area(contour: &[[f32; 2]]) -> f32 {
    let n = contour.len();
    (0..n)
        .map(|i| {
            let a = contour[i];
            let b = contour[(i + 1) % n];
            a[0] * b[1] - b[0] * a[1]
        })
        .sum::<f32>()
        * 0.5
}

/// A clean shape's filled area and the length of all its contours.
pub(super) fn area_and_perimeter(shape: &[Contour]) -> (f32, f32) {
    let mut area = 0.0f64;
    let mut perimeter = 0.0f64;
    for contour in shape {
        let n = contour.len();
        for i in 0..n {
            let (a, b) = (contour[i], contour[(i + 1) % n]);
            area += a[0] * b[1] - b[0] * a[1];
            perimeter += ((b[0] - a[0]).powi(2) + (b[1] - a[1]).powi(2)).sqrt();
        }
    }
    // Holes wind the other way, so the signed sum is the filled area.
    ((area * 0.5).abs() as f32, perimeter as f32)
}

/// The error function, to about 1e-7 (Abramowitz and Stegun 7.1.26).
pub(super) fn erf(x: f32) -> f32 {
    let sign = x.signum();
    let x = f64::from(x.abs());
    let t = 1.0 / (1.0 + 0.327_591_1 * x);
    let poly = t
        * (0.254_829_592
            + t * (-0.284_496_736
                + t * (1.421_413_741 + t * (-1.453_152_027 + t * 1.061_405_429))));
    (sign * (1.0 - poly * (-x * x).exp()) as f32).clamp(-1.0, 1.0)
}

pub(super) fn normalize(v: [f32; 2]) -> [f32; 2] {
    let length = (v[0] * v[0] + v[1] * v[1]).sqrt();
    if length > 1e-12 {
        [v[0] / length, v[1] / length]
    } else {
        [0.0, 0.0]
    }
}

/// Outward miter direction at every vertex of a contour, scaled so moving a
/// vertex by `miter * d` moves both adjacent edges out by `d`.
///
/// `outward_sign` is which side of an edge the filled region is not on: `1`
/// for the right-hand normal `(dy, -dx)`.
pub(super) fn miters(contour: &[[f32; 2]], outward_sign: f32) -> Vec<[f32; 2]> {
    let n = contour.len();
    let normal = |a: [f32; 2], b: [f32; 2]| {
        let d = normalize([b[0] - a[0], b[1] - a[1]]);
        [d[1] * outward_sign, -d[0] * outward_sign]
    };
    (0..n)
        .map(|i| {
            let prev = contour[(i + n - 1) % n];
            let here = contour[i];
            let next = contour[(i + 1) % n];
            let n1 = normal(prev, here);
            let n2 = normal(here, next);
            let denominator = 1.0 + n1[0] * n2[0] + n1[1] * n2[1];
            let miter = if denominator > 1e-3 {
                [(n1[0] + n2[0]) / denominator, (n1[1] + n2[1]) / denominator]
            } else {
                n1
            };
            let length = (miter[0] * miter[0] + miter[1] * miter[1]).sqrt();
            if length > MITER_LIMIT {
                [
                    miter[0] * MITER_LIMIT / length,
                    miter[1] * MITER_LIMIT / length,
                ]
            } else {
                miter
            }
        })
        .collect()
}

/// Which side the filled region of a clean shape is on, from its outer
/// contour: holes run the other way round, so one sign serves the shape.
pub(super) fn shape_outward_sign(shape: &[Contour]) -> f32 {
    let outer: Vec<[f32; 2]> = shape
        .first()
        .map(|contour| contour.iter().map(|p| [p[0] as f32, p[1] as f32]).collect())
        .unwrap_or_default();
    // Positive area: the interior is on the left normal `(-dy, dx)`, so
    // outward is the right one.
    if signed_area(&outer) >= 0.0 {
        1.0
    } else {
        -1.0
    }
}

pub(super) fn tri_bounds(tri: [[f32; 2]; 3]) -> ([f32; 2], [f32; 2]) {
    let min = [
        tri[0][0].min(tri[1][0]).min(tri[2][0]),
        tri[0][1].min(tri[1][1]).min(tri[2][1]),
    ];
    let max = [
        tri[0][0].max(tri[1][0]).max(tri[2][0]),
        tri[0][1].max(tri[1][1]).max(tri[2][1]),
    ];
    (min, max)
}

/// Sutherland–Hodgman against the half-plane left of `a → b` (the inside of
/// a counter-clockwise triangle).
pub(super) fn clip_polygon(polygon: &[PathVertex], a: [f32; 2], b: [f32; 2]) -> Vec<PathVertex> {
    let side = |p: [f32; 2]| cross(a, b, p);
    let mut out = Vec::with_capacity(polygon.len() + 2);
    for i in 0..polygon.len() {
        let current = polygon[i];
        let next = polygon[(i + 1) % polygon.len()];
        let (sc, sn) = (side(current.position), side(next.position));
        if sc >= 0.0 {
            out.push(current);
        }
        if (sc >= 0.0) != (sn >= 0.0) {
            let t = sc / (sc - sn);
            out.push(lerp_vertex(current, next, t));
        }
    }
    out
}

pub(super) fn lerp_vertex(a: PathVertex, b: PathVertex, t: f32) -> PathVertex {
    let mix = |x: f32, y: f32| x + (y - x) * t;
    PathVertex {
        position: [
            mix(a.position[0], b.position[0]),
            mix(a.position[1], b.position[1]),
        ],
        extrude: [
            mix(a.extrude[0], b.extrude[0]),
            mix(a.extrude[1], b.extrude[1]),
        ],
        coverage: mix(a.coverage, b.coverage),
        color: std::array::from_fn(|i| mix(a.color[i], b.color[i])),
        paint_pos: [
            mix(a.paint_pos[0], b.paint_pos[0]),
            mix(a.paint_pos[1], b.paint_pos[1]),
        ],
    }
}
