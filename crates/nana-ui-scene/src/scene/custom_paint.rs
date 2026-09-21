//! Custom paint (Issue #217): what a node's `Painter` recorded, turned into
//! scene primitives.
//!
//! A recording is resolved in node-local logical pixels. Everything that is
//! path geometry — fills, strokes, shadows, and rounded rectangles cut by a
//! path clip — goes through the same route: flatten, let `i_overlay` resolve
//! the fill rule, stroke outline and clip intersection into clean polygons,
//! then tessellate those with `lyon` and add an anti-aliasing fringe. Clipping
//! before tessellation is what keeps clipped edges anti-aliased.
//!
//! Building that geometry is the expensive part, and it depends on nothing but
//! the recording, so it is kept per node and reused for as long as the
//! runtime hands the same recording back. A node that moves, scrolls or is
//! rebuilt for any other reason re-emits primitives from it without
//! re-tessellating anything.

use i_overlay::core::fill_rule::FillRule as OverlayFill;
use i_overlay::core::overlay_rule::OverlayRule;
use i_overlay::float::simplify::SimplifyShape;
use i_overlay::float::single::SingleFloatOverlay;
use i_overlay::mesh::float::outline::offset::OutlineOffset;
use i_overlay::mesh::float::stroke::offset::StrokeOffset;
use i_overlay::mesh::float::style::{
    LineCap as OverlayCap, LineJoin as OverlayJoin, OutlineStyle, StrokeStyle as OverlayStroke,
};
use lyon_tessellation::path::iterator::PathIterator;
use lyon_tessellation::path::{Path as LyonPath, PathEvent};
use lyon_tessellation::{
    BuffersBuilder, FillOptions, FillRule as LyonFill, FillTessellator, FillVertex, VertexBuffers,
};
use nana_ui_runtime::{
    AFFINE_IDENTITY, Affine, BlendMode, FillRule, ImageFit, LineCap, LineJoin, PaintOp, PaintPath,
    PaintPhase, PaintRecording, PathVerb, ResolvedGradient, ResolvedPaint, StrokeStyle,
};

use super::*;

type Point = [f64; 2];
type Contour = Vec<Point>;
type Shapes = Vec<Vec<Contour>>;

/// Curve flattening tolerance in logical px. A 2x display sees 0.1 physical
/// px of error at most.
const TOLERANCE: f32 = 0.05;
/// Longest a miter extrusion may get, in units of the offset. Keeps the AA
/// fringe and shadow band of a spike from shooting across the node.
const MITER_LIMIT: f32 = 4.0;

/// Triangles a node's painter produced, in node-local logical px.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct PathMesh {
    pub vertices: Vec<PathVertex>,
    pub indices: Vec<u32>,
    /// Local bounds of every vertex, before the AA fringe (which reaches one
    /// physical px further out).
    pub bounds: SceneRect,
    /// Colour every fragment from this gradient, evaluated at the vertex's
    /// [`PathVertex::paint_pos`]; the vertex colour then only tints it
    /// (coverage and opacity). `None`: the vertex colour is the colour.
    pub gradient: Option<Arc<ResolvedGradient>>,
}

/// One tessellated vertex.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PathVertex {
    pub position: [f32; 2],
    /// Where this vertex moves for one physical pixel of anti-aliasing fringe,
    /// in local units per physical px. Zero for every vertex that does not
    /// sit on the outside of a fringe.
    pub extrude: [f32; 2],
    /// Linear coverage in `0..=1`. The painter shapes it with a smoothstep,
    /// which turns a linear ramp across `±2σ` into a close fit of a Gaussian
    /// edge — the same ramp serves the one-pixel AA fringe and a blurred
    /// shadow band.
    pub coverage: f32,
    /// Straight (not premultiplied) RGBA.
    pub color: [f32; 4],
    /// Where this vertex sits in the space the gradient was defined in: the
    /// painter's coordinates before its local transform.
    pub paint_pos: [f32; 2],
}

/// A clip the built-in GPU clip can express exactly, in node-local px. Used
/// for what is not tessellated here: text, icons, images and the node's
/// default visual. A path clip it cannot express goes through a masked layer
/// instead.
#[derive(Debug, Clone, PartialEq)]
enum LocalClip {
    /// Axis-aligned rectangle with a uniform corner radius.
    Rounded { rect: SceneRect, radius: f32 },
    /// Up to eight vertices, relative to `bounds`' origin.
    Polygon {
        bounds: SceneRect,
        points: Vec<[f32; 2]>,
    },
}

/// A recorded op with its geometry already built.
#[derive(Debug, Clone, PartialEq)]
enum BuiltOp {
    Mesh(Arc<PathMesh>),
    Quad {
        rect: SceneRect,
        radii: [f32; 4],
        fill: Option<[f32; 4]>,
        border: Option<([f32; 4], f32)>,
        shadow: Option<ComponentElevation>,
        transform: Affine,
    },
    Image {
        rect: SceneRect,
        source: Arc<str>,
        fit: ImageFit,
        radii: [f32; 4],
        opacity: f32,
        clips: Arc<[LocalClip]>,
        transform: Affine,
    },
    Text {
        rect: SceneRect,
        text: Arc<TextStyle>,
        color: [f32; 4],
        clips: Arc<[LocalClip]>,
        transform: Affine,
    },
    Icon {
        rect: SceneRect,
        icon: Icon,
        color: [f32; 4],
        clips: Arc<[LocalClip]>,
        transform: Affine,
    },
    Default {
        clips: Arc<[LocalClip]>,
        transform: Affine,
        opacity: f32,
    },
    /// What follows, up to the matching [`Self::LayerEnd`], composites as one
    /// layer, cut to `clip` (painter-local px) when there is one.
    LayerBegin {
        opacity: f32,
        blend: BlendMode,
        clip: Option<SceneRect>,
    },
    LayerEnd {
        mask: Option<(Arc<PathMesh>, LayerMaskMode)>,
    },
}

/// A text op's content and style.
#[derive(Debug, Clone, PartialEq)]
struct TextStyle {
    content: Arc<str>,
    size: f32,
    weight: Option<u16>,
    italic: bool,
    line_height: Option<f32>,
    wrap: bool,
    max_lines: Option<u16>,
    horizontal: TextHorizontalAlignment,
    vertical: TextVerticalAlignment,
}

/// A recording's geometry, kept per node. See the module docs.
#[derive(Debug, Clone)]
pub(super) struct BuiltPaint {
    recording: Arc<PaintRecording>,
    behind: Arc<[BuiltOp]>,
    over: Arc<[BuiltOp]>,
}

/// The slot namespaces a painted node's own primitives live in. Their paint
/// order is decided by [`custom_paint_stack`], which reads the namespace.
pub(super) const PAINT_BEHIND_PRE: u32 = 0xFFFF_FFF0;
pub(super) const PAINT_BEHIND_POST: u32 = 0xFFFF_FFF1;
pub(super) const PAINT_OVER_PRE: u32 = 0xFFFF_FFF2;
pub(super) const PAINT_OVER_POST: u32 = 0xFFFF_FFF3;

/// The stacking entry a painted node's primitive sorts under, after the
/// node's own prefix (a painted node is always a stacking group).
///
/// Behind-children ops recorded before `draw_default()` sit at the prefix
/// itself, ahead of everything in the group. The default visual and the ops
/// after it come next, at `(i32::MIN, 0)` — still below any child, whatever
/// its z-index, because a child's document order is at least one. Over-children
/// ops sit past every child at `i32::MAX`. Within one entry the slot orders
/// them: every namespace here is above every built-in slot.
pub(super) fn custom_paint_stack(recording: &PaintRecording, slot: u64) -> Option<(i32, usize)> {
    let namespace = (slot >> 32) as u32;
    let default_over = recording.default_phase() == Some(PaintPhase::OverChildren);
    match namespace {
        PAINT_BEHIND_PRE => None,
        PAINT_BEHIND_POST => Some((i32::MIN, 0)),
        PAINT_OVER_PRE => Some((i32::MAX, usize::MAX - 1)),
        PAINT_OVER_POST => Some((i32::MAX, usize::MAX)),
        // The node's built-in primitives: wherever `draw_default()` put them.
        _ if default_over => Some((i32::MAX, usize::MAX)),
        _ => Some((i32::MIN, 0)),
    }
}

impl UiScene {
    /// The geometry of a painted node's recording, built once per recording.
    ///
    /// Called before any of the node's primitives are inserted: paint order
    /// reads whether the node is painted from this map.
    pub(super) fn prepare_custom_paint(
        &mut self,
        id: StableNodeId,
        recording: &Arc<PaintRecording>,
    ) -> BuiltPaint {
        match self.custom_paint.get(&id) {
            Some(built) if Arc::ptr_eq(&built.recording, recording) => built.clone(),
            _ => {
                let built = BuiltPaint {
                    recording: Arc::clone(recording),
                    behind: build_ops(&recording.behind_children).into(),
                    over: build_ops(&recording.over_children).into(),
                };
                self.custom_paint.insert(id, built.clone());
                built
            }
        }
    }

    /// Emit a painted node's own primitives. The built-in ones, if the
    /// recording asked for them, are already in; they get the path clips,
    /// local transform and opacity active at `draw_default()`.
    pub(super) fn emit_custom_paint(
        &mut self,
        node: &ExtractedNode,
        built: &BuiltPaint,
        transform: AffineTransform,
        clips: &Arc<[ClipRegion]>,
        opacity: f32,
        node_order: usize,
    ) {
        let origin = [node.layout.x, node.layout.y];
        for (ops, pre, post) in [
            (&built.behind, PAINT_BEHIND_PRE, PAINT_BEHIND_POST),
            (&built.over, PAINT_OVER_PRE, PAINT_OVER_POST),
        ] {
            let mut namespace = pre;
            for (index, op) in ops.iter().enumerate() {
                let local_clips = |local: &[LocalClip]| -> Arc<[ClipRegion]> {
                    if local.is_empty() {
                        return Arc::clone(clips);
                    }
                    let mut chain = clips.to_vec();
                    chain.extend(
                        local
                            .iter()
                            .map(|clip| clip_region(clip, origin, transform)),
                    );
                    chain.into()
                };
                // A primitive the painter drew under a local transform keeps
                // its layout-space bounds; the transform goes on top of the
                // node's, about the node's origin.
                let under = |local: &Affine| transform.then(about_origin(*local, origin));
                let mut primitive_opacity = opacity;
                let (bounds, kind, primitive_clips, primitive_transform) = match op {
                    BuiltOp::Default {
                        clips: local,
                        transform: local_transform,
                        opacity: op_opacity,
                    } => {
                        namespace = post;
                        let extra: Vec<ClipRegion> = local
                            .iter()
                            .map(|clip| clip_region(clip, origin, transform))
                            .collect();
                        let moved = (*local_transform != AFFINE_IDENTITY)
                            .then(|| about_origin(*local_transform, origin));
                        if !extra.is_empty() || moved.is_some() || *op_opacity != 1.0 {
                            self.adjust_default_paint(node.id, &extra, moved, *op_opacity);
                        }
                        continue;
                    }
                    BuiltOp::LayerBegin {
                        opacity: layer_opacity,
                        blend,
                        clip,
                    } => (
                        clip.map_or(
                            SceneRect {
                                x: origin[0],
                                y: origin[1],
                                width: node.layout.width,
                                height: node.layout.height,
                            },
                            |clip| offset_rect(clip, origin),
                        ),
                        ScenePrimitiveKind::LayerBegin {
                            opacity: *layer_opacity,
                            blend: *blend,
                            clip: clip.map(|clip| offset_rect(clip, origin)),
                        },
                        Arc::clone(clips),
                        transform,
                    ),
                    BuiltOp::LayerEnd { mask } => (
                        mask.as_ref().map_or(
                            SceneRect {
                                x: origin[0],
                                y: origin[1],
                                width: node.layout.width,
                                height: node.layout.height,
                            },
                            |(mesh, _)| offset_rect(mesh.bounds, origin),
                        ),
                        ScenePrimitiveKind::LayerEnd {
                            mask: mask.as_ref().map(|(mesh, mode)| LayerMask {
                                mesh: Arc::clone(mesh),
                                origin,
                                mode: *mode,
                            }),
                        },
                        Arc::clone(clips),
                        transform,
                    ),
                    BuiltOp::Mesh(mesh) => (
                        offset_rect(mesh.bounds, origin),
                        ScenePrimitiveKind::Path {
                            mesh: Arc::clone(mesh),
                            origin,
                        },
                        Arc::clone(clips),
                        transform,
                    ),
                    BuiltOp::Quad {
                        rect,
                        radii,
                        fill,
                        border,
                        shadow,
                        transform: local_transform,
                    } => (
                        offset_rect(*rect, origin),
                        ScenePrimitiveKind::Quad {
                            background: *fill,
                            border_color: border.map(|(color, _)| color),
                            border_width: border.map_or(0.0, |(_, width)| width),
                            corner_radius: *radii,
                            shadow: *shadow,
                            surface: QuadSurfacePaint::default(),
                        },
                        Arc::clone(clips),
                        under(local_transform),
                    ),
                    BuiltOp::Image {
                        rect,
                        source,
                        fit,
                        radii,
                        opacity: op_opacity,
                        clips: local,
                        transform: local_transform,
                    } => {
                        primitive_opacity *= op_opacity;
                        (
                            offset_rect(*rect, origin),
                            image_quad(source, *fit, *radii),
                            local_clips(local),
                            under(local_transform),
                        )
                    }
                    BuiltOp::Text {
                        rect,
                        text,
                        color,
                        clips: local,
                        transform: local_transform,
                    } => (
                        offset_rect(*rect, origin),
                        custom_text(node, text, *color),
                        local_clips(local),
                        under(local_transform),
                    ),
                    BuiltOp::Icon {
                        rect,
                        icon,
                        color,
                        clips: local,
                        transform: local_transform,
                    } => (
                        offset_rect(*rect, origin),
                        ScenePrimitiveKind::Icon {
                            icon: *icon,
                            color: Some(*color),
                        },
                        local_clips(local),
                        under(local_transform),
                    ),
                };
                self.insert_primitive(ScenePrimitive {
                    id: PrimitiveId {
                        node: node.id,
                        slot: collection_slot(namespace, index),
                    },
                    node: node.id,
                    bounds,
                    transform: primitive_transform,
                    clips: primitive_clips,
                    opacity: primitive_opacity,
                    z_index: node.z_index,
                    document_order: node_order,
                    kind,
                });
            }
        }
    }
}

impl UiScene {
    /// Put every built-in primitive this rebuild wrote for `id` under the
    /// painter's state at `draw_default()`: extra clips, a local transform
    /// and an opacity.
    fn adjust_default_paint(
        &mut self,
        id: StableNodeId,
        extra: &[ClipRegion],
        local: Option<AffineTransform>,
        opacity: f32,
    ) {
        let build = self.build;
        let range = PrimitiveId { node: id, slot: 0 }..=PrimitiveId {
            node: id,
            slot: u64::MAX,
        };
        for (slot, held) in self.primitives.range_mut(range) {
            let namespace = (slot.slot >> 32) as u32;
            if held.build != build || namespace >= PAINT_BEHIND_PRE {
                continue;
            }
            if !extra.is_empty() {
                let mut chain = held.primitive.clips.to_vec();
                chain.extend_from_slice(extra);
                held.primitive.clips = chain.into();
            }
            if let Some(local) = local {
                held.primitive.transform = held.primitive.transform.then(local);
            }
            held.primitive.opacity *= opacity;
        }
    }
}

/// `local`, which maps painter coordinates (origin at the node's top-left),
/// as a transform of layout space.
fn about_origin(local: Affine, origin: [f32; 2]) -> AffineTransform {
    let [a, b, c, d, e, f] = local;
    let [ox, oy] = origin;
    // T(origin) · local · T(-origin)
    AffineTransform::from_matrix([
        a,
        b,
        c,
        d,
        e + ox - (a * ox + c * oy),
        f + oy - (b * ox + d * oy),
    ])
}

fn image_quad(source: &Arc<str>, fit: ImageFit, radii: [f32; 4]) -> ScenePrimitiveKind {
    let mut image = BackgroundImage::url_with_fit(
        source.as_ref(),
        match fit {
            ImageFit::Fill => nana_ui_core::BackgroundImageFit::Stretch,
            ImageFit::Contain => nana_ui_core::BackgroundImageFit::Contain,
            ImageFit::Cover => nana_ui_core::BackgroundImageFit::Cover,
            ImageFit::None => nana_ui_core::BackgroundImageFit::Auto,
            ImageFit::ScaleDown => nana_ui_core::BackgroundImageFit::ScaleDown,
        },
    );
    if let BackgroundImage::Url {
        repeat, position, ..
    } = &mut image
    {
        *repeat = nana_ui_core::BackgroundRepeat::NoRepeat;
        *position = nana_ui_core::BackgroundPosition::center();
    }
    ScenePrimitiveKind::Quad {
        background: None,
        border_color: None,
        border_width: 0.0,
        corner_radius: radii,
        shadow: None,
        surface: QuadSurfacePaint {
            content_image: Some(image),
            ..QuadSurfacePaint::default()
        },
    }
}

fn custom_text(node: &ExtractedNode, text: &TextStyle, color: [f32; 4]) -> ScenePrimitiveKind {
    ScenePrimitiveKind::Text {
        content: text.content.to_string(),
        color: Some(color),
        size: text.size,
        weight: text.weight,
        family: node.style.font_family.as_deref().map(str::to_owned),
        line_height: text.line_height.map(nana_ui_core::LineHeightSpec::Absolute),
        letter_spacing: 0.0,
        wrap: text.wrap,
        ellipsis: true,
        max_lines: if text.wrap { text.max_lines } else { Some(1) },
        shaping: TextShaping::Auto,
        horizontal_alignment: text.horizontal,
        vertical_alignment: text.vertical,
        spans: Vec::new(),
        text_shadow: None,
        underline: false,
        line_through: false,
        font_features: Vec::new(),
        italic: text.italic,
        wrap_break: nana_ui_core::TextWrapBreak::default(),
        opentype: SceneTextOpenType::from_computed(&node.style),
        layout: None,
    }
}

fn offset_rect(rect: SceneRect, origin: [f32; 2]) -> SceneRect {
    SceneRect {
        x: rect.x + origin[0],
        y: rect.y + origin[1],
        ..rect
    }
}

fn clip_region(clip: &LocalClip, origin: [f32; 2], transform: AffineTransform) -> ClipRegion {
    match clip {
        LocalClip::Rounded { rect, radius } => ClipRegion {
            bounds: offset_rect(*rect, origin),
            transform,
            corner_radius: *radius,
            polygon_clip: None,
        },
        LocalClip::Polygon { bounds, points } => ClipRegion {
            bounds: offset_rect(*bounds, origin),
            transform,
            corner_radius: 0.0,
            polygon_clip: Some(points.clone()),
        },
    }
}

/// One entry of the clip stack while building: everything pushed so far,
/// intersected.
struct ActiveClip {
    shapes: Shapes,
    /// The clip, triangulated, for cutting meshes that cannot be re-derived
    /// from a polygon (shadow bands carry a coverage ramp).
    triangles: Vec<[[f32; 2]; 3]>,
    /// The same clips as GPU clips, when every one of them is exact.
    /// `None`: content the CPU cannot cut goes through a masked layer.
    local: Option<Arc<[LocalClip]>>,
}

/// What the painter's state says about the op being built.
#[derive(Clone, Copy)]
struct OpState {
    transform: Affine,
    opacity: f32,
}

fn build_ops(ops: &[PaintOp]) -> Vec<BuiltOp> {
    let mut built = Vec::with_capacity(ops.len());
    let mut stack: Vec<ActiveClip> = Vec::new();
    let no_clips: Arc<[LocalClip]> = Arc::from(Vec::new());
    let mut state = OpState {
        transform: AFFINE_IDENTITY,
        opacity: 1.0,
    };
    for op in ops {
        let clip = stack.last();
        let t = state.transform;
        let alpha = state.opacity;
        match op {
            PaintOp::SetTransform(transform) => state.transform = *transform,
            PaintOp::SetOpacity(opacity) => state.opacity = *opacity,
            PaintOp::PushLayer { opacity, blend } => built.push(BuiltOp::LayerBegin {
                opacity: *opacity,
                blend: *blend,
                clip: None,
            }),
            PaintOp::PopLayer => built.push(BuiltOp::LayerEnd { mask: None }),
            PaintOp::PushClip { path } => {
                let own = fill_shapes(path, t);
                let exact = local_clip(path, &own, t);
                let shapes = match clip {
                    Some(outer) => intersect(&own, &outer.shapes),
                    None => own,
                };
                let local = match (clip.map(|outer| outer.local.clone()), exact) {
                    (None, Some(exact)) => Some(Arc::from(vec![exact])),
                    (Some(Some(outer)), Some(exact)) => {
                        let mut chain = outer.to_vec();
                        chain.push(exact);
                        Some(chain.into())
                    }
                    _ => None,
                };
                stack.push(ActiveClip {
                    triangles: triangulate(&shapes),
                    shapes,
                    local,
                });
            }
            PaintOp::PopClip => {
                stack.pop();
            }
            PaintOp::FillPath { path, paint } => {
                // A solid (rounded) rectangle is a quad: nothing to
                // triangulate, and a handful of instance bytes a frame instead
                // of a mesh's worth of vertices.
                if clip.is_none()
                    && let ResolvedPaint::Solid(color) = paint
                    && let Some((rect, radii)) = quad_shape(path)
                {
                    built.push(BuiltOp::Quad {
                        rect,
                        radii,
                        fill: Some(fade(*color, alpha)),
                        border: None,
                        shadow: None,
                        transform: t,
                    });
                    continue;
                }
                push_fill(&mut built, fill_shapes(path, t), paint, state, clip);
            }
            PaintOp::StrokePath {
                path,
                paint,
                stroke,
            } => {
                // Likewise a solid, undashed stroke of one: a border drawn on
                // the rectangle grown by half the width.
                if clip.is_none()
                    && let ResolvedPaint::Solid(color) = paint
                    && let Some((rect, radii)) = quad_shape(path)
                    && let Some(outer) = stroke_quad_radii(radii, stroke)
                {
                    let half = stroke.width * 0.5;
                    built.push(BuiltOp::Quad {
                        rect: outset(rect, half),
                        radii: outer,
                        fill: None,
                        border: Some((fade(*color, alpha), stroke.width)),
                        shadow: None,
                        transform: t,
                    });
                    continue;
                }
                push_fill(
                    &mut built,
                    stroke_shapes(path, stroke, t),
                    paint,
                    state,
                    clip,
                );
            }
            PaintOp::Shadow { path, shadow } => {
                push_shadow(&mut built, fill_shapes(path, t), shadow, alpha, clip);
            }
            PaintOp::RoundedRect {
                rect,
                radii,
                fill,
                border,
                shadow,
            } => {
                let rect = scene_rect(*rect);
                let solid = match fill {
                    None => Some(None),
                    Some(ResolvedPaint::Solid(color)) => Some(Some(fade(*color, alpha))),
                    Some(ResolvedPaint::Gradient(_)) => None,
                };
                // The SDF quad draws a solid box under any affine transform;
                // a gradient or a path clip needs the box as paths, and so
                // does a shadow under more than a translation — the quad
                // would turn and scale it, and a painter's shadows do not.
                let translation = t[..4] == AFFINE_IDENTITY[..4];
                if clip.is_none()
                    && (shadow.is_none() || translation)
                    && let Some(fill) = solid
                {
                    built.push(BuiltOp::Quad {
                        rect,
                        radii: *radii,
                        fill,
                        border: border.map(|(color, width)| (fade(color, alpha), width)),
                        shadow: shadow.map(|shadow| ComponentElevation {
                            color: fade(shadow.color, alpha),
                            ..shadow
                        }),
                        transform: t,
                    });
                    continue;
                }
                let outline = rounded_shapes(rect, *radii, t);
                if let Some(shadow) = shadow {
                    push_shadow(&mut built, outline.clone(), shadow, alpha, clip);
                }
                if let Some(fill) = fill {
                    push_fill(&mut built, outline.clone(), fill, state, clip);
                }
                if let Some((color, width)) = border {
                    let inner = rounded_shapes(
                        SceneRect {
                            x: rect.x + width,
                            y: rect.y + width,
                            width: (rect.width - width * 2.0).max(0.0),
                            height: (rect.height - width * 2.0).max(0.0),
                        },
                        radii.map(|radius| (radius - width).max(0.0)),
                        t,
                    );
                    let ring =
                        outline.overlay(&inner, OverlayRule::Difference, OverlayFill::NonZero);
                    push_fill(&mut built, ring, &ResolvedPaint::Solid(*color), state, clip);
                }
            }
            PaintOp::Image {
                rect,
                source,
                fit,
                radii,
            } => {
                let rect = scene_rect(*rect);
                clipped(&mut built, clip, &no_clips, |clips| BuiltOp::Image {
                    rect,
                    source: Arc::clone(source),
                    fit: *fit,
                    radii: *radii,
                    opacity: alpha,
                    clips,
                    transform: t,
                });
            }
            PaintOp::FillImage {
                path,
                rect,
                source,
                fit,
            } => {
                let shapes = fill_shapes(path, t);
                let shapes = match clip {
                    Some(outer) => intersect(&shapes, &outer.shapes),
                    None => shapes,
                };
                let Some(area) = shapes_bounds(&shapes) else {
                    continue;
                };
                let area = outset(area, 1.0);
                built.push(BuiltOp::LayerBegin {
                    opacity: 1.0,
                    blend: BlendMode::Normal,
                    clip: Some(area),
                });
                built.push(BuiltOp::Image {
                    rect: scene_rect(*rect),
                    source: Arc::clone(source),
                    fit: *fit,
                    radii: [0.0; 4],
                    opacity: alpha,
                    clips: Arc::clone(&no_clips),
                    transform: t,
                });
                built.push(BuiltOp::LayerEnd {
                    mask: erase_outside(area, &shapes).map(|mesh| (mesh, LayerMaskMode::Erase)),
                });
            }
            PaintOp::Text {
                rect,
                content,
                paint,
                size,
                weight,
                italic,
                line_height,
                wrap,
                max_lines,
                horizontal,
                vertical,
            } => {
                let rect = scene_rect(*rect);
                let text = Arc::new(TextStyle {
                    content: Arc::clone(content),
                    size: *size,
                    weight: *weight,
                    italic: *italic,
                    line_height: *line_height,
                    wrap: *wrap,
                    max_lines: *max_lines,
                    horizontal: *horizontal,
                    vertical: *vertical,
                });
                // Glyphs can reach past their box: italics, a clipped
                // ellipsis. A tint must reach them too.
                let reach = size * 0.5;
                tinted(&mut built, paint, rect, reach, state, |color| {
                    clipped_op(clip, &no_clips, |clips| BuiltOp::Text {
                        rect,
                        text: Arc::clone(&text),
                        color,
                        clips,
                        transform: t,
                    })
                });
            }
            PaintOp::Icon { rect, icon, paint } => {
                let rect = scene_rect(*rect);
                tinted(&mut built, paint, rect, 1.0, state, |color| {
                    clipped_op(clip, &no_clips, |clips| BuiltOp::Icon {
                        rect,
                        icon: *icon,
                        color,
                        clips,
                        transform: t,
                    })
                });
            }
            PaintOp::DrawDefault => {
                let ops = clipped_op(clip, &no_clips, |clips| BuiltOp::Default {
                    clips,
                    transform: t,
                    opacity: alpha,
                });
                if ops.is_empty() {
                    // An empty clip hides the default visual, but it still has
                    // to be placed: the node emits it either way, and what
                    // follows draws over it.
                    built.push(BuiltOp::Default {
                        clips: Arc::clone(&no_clips),
                        transform: t,
                        opacity: 0.0,
                    });
                } else {
                    built.extend(ops);
                }
            }
        }
    }
    built
}

/// The rectangle and corner radii of a path that is one (rounded)
/// rectangle with room to draw.
fn quad_shape(path: &PaintPath) -> Option<(SceneRect, [f32; 4])> {
    let (rect, radii) = path.as_rounded_rect_corners()?;
    let finite = [rect.x, rect.y, rect.width, rect.height]
        .iter()
        .chain(radii.iter())
        .all(|v| v.is_finite());
    (finite && rect.width > 0.0 && rect.height > 0.0).then(|| (scene_rect(rect), radii))
}

/// The outer corner radii of a stroke along a rounded rectangle, as the
/// border of a quad grown by half the stroke — or `None` when a quad border
/// cannot draw it: a dash, or a bevelled (or miter-limited) sharp corner.
fn stroke_quad_radii(radii: [f32; 4], stroke: &StrokeStyle) -> Option<[f32; 4]> {
    if dash_pattern(&stroke.dash).is_some()
        || !stroke.width.is_finite()
        || stroke.width <= 0.0
        || stroke.width > MAX_LENGTH
    {
        return None;
    }
    let half = stroke.width * 0.5;
    // A square corner's miter reaches √2 widths: a lower limit bevels it.
    let square_miter = stroke.miter_limit >= std::f32::consts::SQRT_2;
    let mut outer = [0.0; 4];
    for (slot, radius) in outer.iter_mut().zip(radii) {
        *slot = if radius > 0.0 {
            // Round, miter and bevel joins all follow the arc.
            radius + half
        } else {
            match stroke.join {
                LineJoin::Miter if square_miter => 0.0,
                LineJoin::Round => half,
                _ => return None,
            }
        };
    }
    Some(outer)
}

/// Straight RGBA with its alpha scaled.
fn fade([r, g, b, a]: [f32; 4], alpha: f32) -> [f32; 4] {
    [r, g, b, a * alpha]
}

fn outset(rect: SceneRect, by: f32) -> SceneRect {
    SceneRect {
        x: rect.x - by,
        y: rect.y - by,
        width: rect.width + by * 2.0,
        height: rect.height + by * 2.0,
    }
}

/// An op the CPU cannot cut, under the active clip: with the exact GPU clips
/// when there are some, or wrapped in a layer masked to the clip otherwise.
fn clipped(
    built: &mut Vec<BuiltOp>,
    clip: Option<&ActiveClip>,
    no_clips: &Arc<[LocalClip]>,
    op: impl FnOnce(Arc<[LocalClip]>) -> BuiltOp,
) {
    built.extend(clipped_op(clip, no_clips, op));
}

fn clipped_op(
    clip: Option<&ActiveClip>,
    no_clips: &Arc<[LocalClip]>,
    op: impl FnOnce(Arc<[LocalClip]>) -> BuiltOp,
) -> Vec<BuiltOp> {
    let Some(clip) = clip else {
        return vec![op(Arc::clone(no_clips))];
    };
    if let Some(local) = &clip.local {
        return vec![op(Arc::clone(local))];
    }
    let Some(area) = shapes_bounds(&clip.shapes) else {
        // An empty clip shows nothing.
        return Vec::new();
    };
    let area = outset(area, 1.0);
    vec![
        BuiltOp::LayerBegin {
            opacity: 1.0,
            blend: BlendMode::Normal,
            clip: Some(area),
        },
        op(Arc::clone(no_clips)),
        BuiltOp::LayerEnd {
            mask: erase_outside(area, &clip.shapes).map(|mesh| (mesh, LayerMaskMode::Erase)),
        },
    ]
}

/// Glyph-like content in `paint`: a solid colour goes straight onto it; a
/// gradient paints the content white in a layer, then tints it.
fn tinted(
    built: &mut Vec<BuiltOp>,
    paint: &ResolvedPaint,
    rect: SceneRect,
    reach: f32,
    state: OpState,
    content: impl FnOnce([f32; 4]) -> Vec<BuiltOp>,
) {
    match paint {
        ResolvedPaint::Solid(color) => built.extend(content(fade(*color, state.opacity))),
        ResolvedPaint::Gradient(gradient) => {
            let over = outset(rect, reach);
            let mut area = PaintPath::new();
            area.rect(LayoutBox {
                x: over.x,
                y: over.y,
                width: over.width,
                height: over.height,
            });
            let mask = gradient_mesh(fill_shapes(&area, state.transform), gradient, state);
            built.push(BuiltOp::LayerBegin {
                opacity: 1.0,
                blend: BlendMode::Normal,
                clip: None,
            });
            built.extend(content([1.0; 4]));
            built.push(BuiltOp::LayerEnd {
                mask: mask.map(|mesh| (mesh, LayerMaskMode::Tint)),
            });
        }
    }
}

/// Coverage of everything in `area` outside `shapes`, for erasing a layer
/// down to the clip.
fn erase_outside(area: SceneRect, shapes: &Shapes) -> Option<Arc<PathMesh>> {
    let bounds = rect_shapes(area);
    let outside = if shapes.is_empty() {
        bounds
    } else {
        bounds.overlay(shapes, OverlayRule::Difference, OverlayFill::NonZero)
    };
    let mut mesh = MeshBuilder::default();
    mesh.fill(&outside, [0.0, 0.0, 0.0, 1.0]);
    mesh.finish().map(Arc::new)
}

fn rect_shapes(rect: SceneRect) -> Shapes {
    let (x0, y0) = (f64::from(rect.x), f64::from(rect.y));
    let (x1, y1) = (x0 + f64::from(rect.width), y0 + f64::from(rect.height));
    vec![vec![vec![[x0, y0], [x1, y0], [x1, y1], [x0, y1]]]].simplify_shape(OverlayFill::NonZero)
}

fn gradient_mesh(
    shapes: Shapes,
    gradient: &Arc<ResolvedGradient>,
    state: OpState,
) -> Option<Arc<PathMesh>> {
    let mut mesh = MeshBuilder::default();
    mesh.fill(&shapes, [1.0, 1.0, 1.0, state.opacity]);
    if let Some(inverse) = affine_inverse(state.transform) {
        for vertex in &mut mesh.vertices {
            vertex.paint_pos = affine_apply(inverse, vertex.position);
        }
    }
    let mut mesh = mesh.finish()?;
    mesh.gradient = Some(Arc::clone(gradient));
    Some(Arc::new(mesh))
}

fn push_fill(
    built: &mut Vec<BuiltOp>,
    shapes: Shapes,
    paint: &ResolvedPaint,
    state: OpState,
    clip: Option<&ActiveClip>,
) {
    if paint.is_invisible() || state.opacity <= 0.0 {
        return;
    }
    let shapes = match clip {
        Some(clip) => intersect(&shapes, &clip.shapes),
        None => shapes,
    };
    match paint {
        ResolvedPaint::Solid(color) => {
            let mut mesh = MeshBuilder::default();
            mesh.fill(&shapes, fade(*color, state.opacity));
            if let Some(mesh) = mesh.finish() {
                built.push(BuiltOp::Mesh(Arc::new(mesh)));
            }
        }
        ResolvedPaint::Gradient(gradient) => {
            if let Some(mesh) = gradient_mesh(shapes, gradient, state) {
                built.push(BuiltOp::Mesh(mesh));
            }
        }
    }
}

fn push_shadow(
    built: &mut Vec<BuiltOp>,
    shapes: Shapes,
    shadow: &ComponentElevation,
    alpha: f32,
    clip: Option<&ActiveClip>,
) {
    let color = fade(shadow.color, alpha);
    let lengths = [
        shadow.offset_x,
        shadow.offset_y,
        shadow.blur_radius,
        shadow.spread_radius,
    ];
    if color[3].is_nan() || color[3] <= 0.0 || lengths.iter().any(|length| !length.is_finite()) {
        return;
    }
    // Past a screen's reach a blur or spread only costs geometry.
    let shadow = ComponentElevation {
        color,
        blur_radius: shadow.blur_radius.min(MAX_LENGTH),
        spread_radius: shadow.spread_radius.clamp(-MAX_LENGTH, MAX_LENGTH),
        ..*shadow
    };
    let mut mesh = MeshBuilder::default();
    if shadow.inset {
        mesh.inset_shadow(shapes, &shadow);
    } else {
        mesh.shadow(shapes, &shadow, true);
    }
    if let Some(clip) = clip {
        mesh.clip_to(&clip.triangles);
    }
    if let Some(mesh) = mesh.finish() {
        built.push(BuiltOp::Mesh(Arc::new(mesh)));
    }
}

fn affine_apply(t: Affine, p: [f32; 2]) -> [f32; 2] {
    let [a, b, c, d, e, f] = t;
    [a * p[0] + c * p[1] + e, b * p[0] + d * p[1] + f]
}

fn affine_inverse(t: Affine) -> Option<Affine> {
    let [a, b, c, d, e, f] = t;
    let det = a * d - b * c;
    if !det.is_finite() || det.abs() < 1e-12 {
        return None;
    }
    let (ia, ib, ic, id) = (d / det, -b / det, -c / det, a / det);
    Some([ia, ib, ic, id, -(ia * e + ic * f), -(ib * e + id * f)])
}

/// How much `t` stretches lengths at most, for flattening curves in the
/// space they are drawn in.
fn affine_scale(t: Affine) -> f32 {
    let [a, b, c, d, _, _] = t;
    let fro2 = a * a + b * b + c * c + d * d;
    let det = a * d - b * c;
    let disc = (fro2 * fro2 - 4.0 * det * det).max(0.0);
    ((fro2 + disc.sqrt()) * 0.5).max(0.0).sqrt()
}

/// The path as a GPU clip, when the GPU clip can express it exactly: a
/// (rounded) rectangle or a polygon of up to eight vertices.
fn local_clip(path: &PaintPath, shapes: &Shapes, t: Affine) -> Option<LocalClip> {
    // A translation and a uniform positive scale keep a rounded rectangle one.
    let [a, b, c, d, e, f] = t;
    if b == 0.0
        && c == 0.0
        && a > 0.0
        && (a - d).abs() < 1e-6
        && let Some((rect, radius)) = path.as_rounded_rect()
    {
        return Some(LocalClip::Rounded {
            rect: SceneRect {
                x: rect.x * a + e,
                y: rect.y * a + f,
                width: rect.width.max(0.0) * a,
                height: rect.height.max(0.0) * a,
            },
            radius: radius * a,
        });
    }
    let bounds = shapes_bounds(shapes)?;
    if let [shape] = shapes.as_slice()
        && let [contour] = shape.as_slice()
        && (3..=8).contains(&contour.len())
    {
        return Some(LocalClip::Polygon {
            bounds,
            points: contour
                .iter()
                .map(|p| [p[0] as f32 - bounds.x, p[1] as f32 - bounds.y])
                .collect(),
        });
    }
    None
}

fn scene_rect(rect: LayoutBox) -> SceneRect {
    SceneRect {
        x: rect.x,
        y: rect.y,
        width: rect.width.max(0.0),
        height: rect.height.max(0.0),
    }
}

fn shapes_bounds(shapes: &Shapes) -> Option<SceneRect> {
    let mut points = shapes.iter().flatten().flatten();
    let first = points.next()?;
    let (mut min, mut max) = (*first, *first);
    for p in points {
        min = [min[0].min(p[0]), min[1].min(p[1])];
        max = [max[0].max(p[0]), max[1].max(p[1])];
    }
    Some(SceneRect {
        x: min[0] as f32,
        y: min[1] as f32,
        width: (max[0] - min[0]) as f32,
        height: (max[1] - min[1]) as f32,
    })
}

// ---------------------------------------------------------------------------
// Geometry
// ---------------------------------------------------------------------------

fn lyon_point(p: [f32; 2]) -> lyon_tessellation::math::Point {
    lyon_tessellation::math::point(p[0], p[1])
}

fn lyon_path(path: &PaintPath, t: Affine) -> LyonPath {
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
fn path_is_drawable(path: &PaintPath, t: Affine) -> bool {
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
const MAX_COORDINATE: f32 = 1.0e6;

/// The longest stroke width, blur or spread drawn as asked.
const MAX_LENGTH: f32 = 1.0e4;

/// Sub-paths as polylines under `t`, each with whether it was closed.
/// `tolerance` is in the space `t` maps to.
fn flatten(path: &PaintPath, t: Affine, tolerance: f32) -> Vec<(Contour, bool)> {
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

fn push_point(contour: &mut Contour, point: Point) {
    if point[0].is_finite()
        && point[1].is_finite()
        && contour.last().is_none_or(|last| !same(*last, point))
    {
        contour.push(point);
    }
}

fn same(a: Point, b: Point) -> bool {
    (a[0] - b[0]).abs() < 1e-4 && (a[1] - b[1]).abs() < 1e-4
}

/// The region a path fills, as clean polygons: outer contours one way round,
/// holes the other, nothing overlapping.
fn fill_shapes(path: &PaintPath, t: Affine) -> Shapes {
    let contours: Vec<Contour> = flatten(path, t, TOLERANCE)
        .into_iter()
        .map(|(contour, _)| contour)
        .filter(|contour| contour.len() >= 3)
        .collect();
    if contours.is_empty() {
        return Vec::new();
    }
    let rule = match path.fill_rule() {
        FillRule::NonZero => OverlayFill::NonZero,
        FillRule::EvenOdd => OverlayFill::EvenOdd,
    };
    contours.simplify_shape(rule)
}

/// The region a stroke covers, as clean polygons.
///
/// The stroke — width, joins, caps, dashes — is laid out in the painter's
/// coordinates before its local transform, then the outline is transformed:
/// a stroke scales with the transform, as on a Canvas.
fn stroke_shapes(path: &PaintPath, stroke: &StrokeStyle, t: Affine) -> Shapes {
    if !stroke.width.is_finite() || stroke.width <= 0.0 || !path_is_drawable(path, t) {
        return Vec::new();
    }
    let cap = |cap: LineCap| match cap {
        LineCap::Butt => OverlayCap::Butt,
        LineCap::Round => OverlayCap::Round(0.1),
        LineCap::Square => OverlayCap::Square,
    };
    let style = OverlayStroke::new(f64::from(stroke.width.min(MAX_LENGTH)))
        .start_cap(cap(stroke.cap))
        .end_cap(cap(stroke.cap))
        .line_join(match stroke.join {
            // A miter longer than `limit` widths is where the corner's
            // interior angle drops below 2·asin(1 / limit); bevel there.
            LineJoin::Miter => {
                let limit = if stroke.miter_limit.is_finite() {
                    stroke.miter_limit.max(1.0)
                } else {
                    10.0
                };
                OverlayJoin::Miter(2.0 * (1.0 / f64::from(limit)).asin())
            }
            LineJoin::Round => OverlayJoin::Round(0.1),
            LineJoin::Bevel => OverlayJoin::Bevel,
        });
    let tolerance = TOLERANCE / affine_scale(t).max(1e-3);
    let dash = dash_pattern(&stroke.dash);
    let mut contours: Vec<Contour> = Vec::new();
    for (points, closed) in flatten(path, AFFINE_IDENTITY, tolerance) {
        if points.len() < 2 {
            continue;
        }
        match &dash {
            Some(pattern) if !too_many_dashes(&points, closed, pattern) => {
                for piece in dashes(&points, closed, pattern, stroke.dash_offset) {
                    if piece.len() >= 2 {
                        for shape in piece.stroke(style.clone(), false) {
                            contours.extend(shape);
                        }
                    }
                }
            }
            _ => {
                for shape in points.stroke(style.clone(), closed) {
                    contours.extend(shape);
                }
            }
        }
    }
    if contours.is_empty() {
        return Vec::new();
    }
    // Sub-paths and dashes overlap each other; one union keeps a translucent
    // stroke from darkening where they cross.
    let mut shapes = contours.simplify_shape(OverlayFill::NonZero);
    if t != AFFINE_IDENTITY {
        let flips = t[0] * t[3] - t[1] * t[2] < 0.0;
        for shape in &mut shapes {
            for contour in shape.iter_mut() {
                for point in contour.iter_mut() {
                    let [x, y] = affine_apply(t, [point[0] as f32, point[1] as f32]);
                    *point = [f64::from(x), f64::from(y)];
                }
                // A mirroring transform reverses every contour; keep outer
                // contours and holes running the way the fill expects.
                if flips {
                    contour.reverse();
                }
            }
        }
    }
    shapes
}

/// A usable dash pattern: `None` for a solid stroke.
fn dash_pattern(dash: &[f32]) -> Option<Vec<f64>> {
    if dash.is_empty()
        || dash
            .iter()
            .any(|length| !length.is_finite() || *length < 0.0)
    {
        return None;
    }
    let mut pattern: Vec<f64> = dash.iter().map(|length| f64::from(*length)).collect();
    if pattern.len() % 2 == 1 {
        pattern.extend_from_within(..);
    }
    (pattern.iter().sum::<f64>() > 1e-6).then_some(pattern)
}

/// Past this many dashes a stroke is drawn solid: the pieces would be
/// sub-pixel, and cutting and outlining millions of them would stall the frame.
const MAX_DASHES: f64 = 10_000.0;

fn too_many_dashes(points: &Contour, closed: bool, pattern: &[f64]) -> bool {
    let mut length: f64 = points
        .windows(2)
        .map(|s| ((s[1][0] - s[0][0]).powi(2) + (s[1][1] - s[0][1]).powi(2)).sqrt())
        .sum();
    if closed && let (Some(first), Some(last)) = (points.first(), points.last()) {
        length += ((first[0] - last[0]).powi(2) + (first[1] - last[1]).powi(2)).sqrt();
    }
    let period: f64 = pattern.iter().sum();
    let dashes = length / period;
    dashes.is_nan() || dashes > MAX_DASHES
}

/// The "on" pieces of a polyline under a dash pattern, as open polylines.
fn dashes(points: &Contour, closed: bool, pattern: &[f64], offset: f32) -> Vec<Contour> {
    let mut path = points.clone();
    if closed {
        path.push(points[0]);
    }
    let period: f64 = pattern.iter().sum();
    // Where in the pattern the path starts.
    let mut phase = f64::from(offset).rem_euclid(period);
    let mut index = 0;
    while phase >= pattern[index] {
        phase -= pattern[index];
        index = (index + 1) % pattern.len();
    }
    let mut left = pattern[index] - phase;
    let mut on = index % 2 == 0;
    let mut pieces = Vec::new();
    let mut current: Contour = if on { vec![path[0]] } else { Vec::new() };
    for segment in path.windows(2) {
        let (mut from, to) = (segment[0], segment[1]);
        let mut length = ((to[0] - from[0]).powi(2) + (to[1] - from[1]).powi(2)).sqrt();
        while length > left {
            let k = left / length;
            let cut = [
                from[0] + (to[0] - from[0]) * k,
                from[1] + (to[1] - from[1]) * k,
            ];
            if on {
                current.push(cut);
                pieces.push(std::mem::take(&mut current));
            } else {
                current = vec![cut];
            }
            on = !on;
            length -= left;
            from = cut;
            index = (index + 1) % pattern.len();
            left = pattern[index];
        }
        left -= length;
        if on {
            current.push(to);
        }
    }
    if on && current.len() >= 2 {
        pieces.push(current);
    }
    pieces
}

fn intersect(subject: &Shapes, clip: &Shapes) -> Shapes {
    if subject.is_empty() || clip.is_empty() {
        return Vec::new();
    }
    subject.overlay(clip, OverlayRule::Intersect, OverlayFill::NonZero)
}

fn rounded_shapes(rect: SceneRect, radii: [f32; 4], t: Affine) -> Shapes {
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

fn triangulate(shapes: &Shapes) -> Vec<[[f32; 2]; 3]> {
    let Some((vertices, indices)) = tessellate(shapes.iter().flatten()) else {
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

/// Interior triangles of a set of closed contours, even-odd.
fn tessellate<'a>(
    contours: impl Iterator<Item = &'a Contour>,
) -> Option<(Vec<[f32; 2]>, Vec<u32>)> {
    let mut builder = LyonPath::builder();
    let mut any = false;
    for contour in contours {
        let mut points = contour.iter();
        let Some(first) = points.next() else {
            continue;
        };
        if contour.len() < 3 {
            continue;
        }
        builder.begin(lyon_point([first[0] as f32, first[1] as f32]));
        for p in points {
            builder.line_to(lyon_point([p[0] as f32, p[1] as f32]));
        }
        builder.end(true);
        any = true;
    }
    if !any {
        return None;
    }
    let path = builder.build();
    let mut buffers: VertexBuffers<[f32; 2], u32> = VertexBuffers::new();
    FillTessellator::new()
        .tessellate_path(
            &path,
            &FillOptions::tolerance(TOLERANCE).with_fill_rule(LyonFill::EvenOdd),
            &mut BuffersBuilder::new(&mut buffers, |vertex: FillVertex| {
                vertex.position().to_array()
            }),
        )
        .ok()?;
    if buffers.indices.is_empty() {
        return None;
    }
    Some((buffers.vertices, buffers.indices))
}

fn cross(a: [f32; 2], b: [f32; 2], c: [f32; 2]) -> f32 {
    (b[0] - a[0]) * (c[1] - a[1]) - (b[1] - a[1]) * (c[0] - a[0])
}

fn signed_area(contour: &[[f32; 2]]) -> f32 {
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
fn area_and_perimeter(shape: &[Contour]) -> (f32, f32) {
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
fn erf(x: f32) -> f32 {
    let sign = x.signum();
    let x = f64::from(x.abs());
    let t = 1.0 / (1.0 + 0.327_591_1 * x);
    let poly = t
        * (0.254_829_592
            + t * (-0.284_496_736
                + t * (1.421_413_741 + t * (-1.453_152_027 + t * 1.061_405_429))));
    (sign * (1.0 - poly * (-x * x).exp()) as f32).clamp(-1.0, 1.0)
}

fn normalize(v: [f32; 2]) -> [f32; 2] {
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
fn miters(contour: &[[f32; 2]], outward_sign: f32) -> Vec<[f32; 2]> {
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
fn shape_outward_sign(shape: &[Contour]) -> f32 {
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

#[derive(Default)]
struct MeshBuilder {
    vertices: Vec<PathVertex>,
    indices: Vec<u32>,
}

impl MeshBuilder {
    fn vertex(
        &mut self,
        position: [f32; 2],
        extrude: [f32; 2],
        coverage: f32,
        color: [f32; 4],
    ) -> u32 {
        let index = self.vertices.len() as u32;
        self.vertices.push(PathVertex {
            position,
            extrude,
            coverage,
            color,
            paint_pos: position,
        });
        index
    }

    /// Solid interior plus a one-physical-pixel fringe outside every edge.
    fn fill(&mut self, shapes: &Shapes, color: [f32; 4]) {
        if color[3] <= 0.0 {
            return;
        }
        for shape in shapes {
            let Some((vertices, indices)) = tessellate(shape.iter()) else {
                continue;
            };
            let base = self.vertices.len() as u32;
            for position in vertices {
                self.vertex(position, [0.0, 0.0], 1.0, color);
            }
            self.indices
                .extend(indices.iter().map(|index| base + index));
            let sign = shape_outward_sign(shape);
            for contour in shape {
                let points: Vec<[f32; 2]> =
                    contour.iter().map(|p| [p[0] as f32, p[1] as f32]).collect();
                if points.len() < 3 {
                    continue;
                }
                let miters = miters(&points, sign);
                self.band(
                    &points,
                    |i| (points[i], [0.0, 0.0]),
                    |i| (points[i], miters[i]),
                    color,
                );
            }
        }
    }

    /// A strip between an inner and an outer ring, coverage 1 → 0.
    fn band(
        &mut self,
        points: &[[f32; 2]],
        inner: impl Fn(usize) -> ([f32; 2], [f32; 2]),
        outer: impl Fn(usize) -> ([f32; 2], [f32; 2]),
        color: [f32; 4],
    ) {
        let n = points.len();
        let base = self.vertices.len() as u32;
        for i in 0..n {
            let (position, extrude) = inner(i);
            self.vertex(position, extrude, 1.0, color);
            let (position, extrude) = outer(i);
            self.vertex(position, extrude, 0.0, color);
        }
        for i in 0..n as u32 {
            let j = (i + 1) % n as u32;
            let (in_i, out_i, in_j, out_j) = (
                base + 2 * i,
                base + 2 * i + 1,
                base + 2 * j,
                base + 2 * j + 1,
            );
            self.indices.extend([in_i, out_i, out_j, in_i, out_j, in_j]);
        }
    }

    /// CSS `box-shadow` along the outline: spread, offset, then a coverage
    /// band `±blur` wide around the edge.
    ///
    /// The band is a strip between an inner and an outer ring that correspond
    /// vertex for vertex, so along a straight edge every quad is an exact
    /// linear ramp in distance to the outline, which the painter's smoothstep
    /// turns into a Gaussian-like edge. Where the outline curves tighter than
    /// the blur, the ring on the inside of the curve would fold back over
    /// itself; there it stops at the local radius of curvature instead, which
    /// only sharpens that corner's falloff a little.
    /// An outer shadow of `shapes`, and the outer edge of its band: the
    /// region it reaches. `fade_thin` lowers the peak of a shape thinner
    /// than the blur, as a real Gaussian would.
    fn shadow(
        &mut self,
        mut shapes: Shapes,
        shadow: &ComponentElevation,
        fade_thin: bool,
    ) -> Shapes {
        if shapes.is_empty() {
            return shapes;
        }
        if shadow.spread_radius.abs() > 0.01 {
            shapes = shapes.outline(
                &OutlineStyle::new(f64::from(shadow.spread_radius))
                    .line_join(OverlayJoin::Round(0.1)),
            );
        }
        let offset = [f64::from(shadow.offset_x), f64::from(shadow.offset_y)];
        for point in shapes.iter_mut().flatten().flatten() {
            point[0] += offset[0];
            point[1] += offset[1];
        }
        let blur = shadow.blur_radius.max(0.0);
        if blur < 0.5 {
            self.fill(&shapes, shadow.color);
            return shapes;
        }
        let mut reach: Shapes = Vec::new();
        for shape in &shapes {
            let Some(bounds) = shapes_bounds(&vec![shape.clone()]) else {
                continue;
            };
            // Past half its narrow side the inner ring would turn inside out.
            let mut inset = blur.min(bounds.width.min(bounds.height) * 0.5);
            let (area, perimeter) = area_and_perimeter(shape);
            if shape.len() > 1 && perimeter > 0.0 {
                // A hole's inner ring runs toward the outline's: past half
                // the shape's thickness (2·area/perimeter for a thin one)
                // they would cross and fill between them solid.
                inset = inset.min(area / perimeter);
            }
            // A shape thinner than its blur never reaches full strength: a
            // Gaussian over a strip `t` wide peaks at erf(t / (2√2·σ)), with
            // σ = blur / 2. 4·area/perimeter is the width of a strip, disc or
            // square, and at most twice it for anything else.
            let thickness = if perimeter > 0.0 {
                bounds.width.min(bounds.height).min(4.0 * area / perimeter)
            } else {
                0.0
            };
            let peak = if fade_thin {
                erf(thickness / (std::f32::consts::SQRT_2 * blur))
            } else {
                1.0
            };
            let color = [
                shadow.color[0],
                shadow.color[1],
                shadow.color[2],
                shadow.color[3] * peak,
            ];
            let sign = shape_outward_sign(shape);
            let mut inner_rings: Vec<Contour> = Vec::new();
            let mut outer_rings: Vec<Contour> = Vec::new();
            for contour in shape {
                let points: Vec<[f32; 2]> =
                    contour.iter().map(|p| [p[0] as f32, p[1] as f32]).collect();
                if points.len() < 3 {
                    continue;
                }
                let miters = miters(&points, sign);
                let reach = ring_reach(&points, &miters, inset, blur);
                let inner: Vec<[f32; 2]> = (0..points.len())
                    .map(|i| {
                        let d = reach[i].0;
                        [
                            points[i][0] - miters[i][0] * d,
                            points[i][1] - miters[i][1] * d,
                        ]
                    })
                    .collect();
                let outer: Vec<[f32; 2]> = (0..points.len())
                    .map(|i| {
                        let d = reach[i].1;
                        [
                            points[i][0] + miters[i][0] * d,
                            points[i][1] + miters[i][1] * d,
                        ]
                    })
                    .collect();
                inner_rings.push(
                    inner
                        .iter()
                        .map(|p| [f64::from(p[0]), f64::from(p[1])])
                        .collect(),
                );
                outer_rings.push(
                    outer
                        .iter()
                        .map(|p| [f64::from(p[0]), f64::from(p[1])])
                        .collect(),
                );
                self.band(
                    &points,
                    |i| (inner[i], [0.0, 0.0]),
                    |i| (outer[i], [0.0, 0.0]),
                    color,
                );
            }
            if let Some((vertices, indices)) = tessellate(inner_rings.iter()) {
                let base = self.vertices.len() as u32;
                for position in vertices {
                    self.vertex(position, [0.0, 0.0], 1.0, color);
                }
                self.indices
                    .extend(indices.iter().map(|index| base + index));
            }
            reach.push(outer_rings);
        }
        reach
    }

    /// CSS `box-shadow: inset` inside the outline: dark where the outline,
    /// offset and shrunk by the spread, does not reach, fading over `±blur`
    /// at its edge, and cut to the outline.
    fn inset_shadow(&mut self, shapes: Shapes, shadow: &ComponentElevation) {
        if shapes.is_empty() {
            return;
        }
        let clip = triangulate(&shapes);
        let mut hole = shapes.clone();
        for point in hole.iter_mut().flatten().flatten() {
            point[0] += f64::from(shadow.offset_x);
            point[1] += f64::from(shadow.offset_y);
        }
        if shadow.spread_radius.abs() > 0.01 {
            hole = hole.outline(
                &OutlineStyle::new(-f64::from(shadow.spread_radius))
                    .line_join(OverlayJoin::Round(0.1)),
            );
        }
        let blur = shadow.blur_radius.max(0.0);
        if blur < 0.5 || hole.is_empty() {
            let dark = if hole.is_empty() {
                shapes
            } else {
                shapes.overlay(&hole, OverlayRule::Difference, OverlayFill::NonZero)
            };
            self.fill(&dark, shadow.color);
        } else {
            // The band around the hole, dark side out.
            let start = self.vertices.len();
            let plain = ComponentElevation {
                offset_x: 0.0,
                offset_y: 0.0,
                spread_radius: 0.0,
                inset: false,
                ..*shadow
            };
            // A thin hole still lets its full darkness reach the outline.
            let reach = self.shadow(hole, &plain, false);
            for vertex in &mut self.vertices[start..] {
                vertex.coverage = 1.0 - vertex.coverage;
            }
            // And solid past the band: past its actual outer edge, whose
            // corners are mitred, so the two never cover a point twice.
            let grown = reach.simplify_shape(OverlayFill::NonZero);
            let beyond = shapes.overlay(&grown, OverlayRule::Difference, OverlayFill::NonZero);
            for shape in &beyond {
                if let Some((vertices, indices)) = tessellate(shape.iter()) {
                    let base = self.vertices.len() as u32;
                    for position in vertices {
                        self.vertex(position, [0.0, 0.0], 1.0, shadow.color);
                    }
                    self.indices
                        .extend(indices.iter().map(|index| base + index));
                }
            }
        }
        self.clip_to(&clip);
    }

    /// Cut every triangle to the clip triangles, interpolating attributes.
    fn clip_to(&mut self, clip: &[[[f32; 2]; 3]]) {
        let vertices = std::mem::take(&mut self.vertices);
        let indices = std::mem::take(&mut self.indices);
        for tri in indices.as_chunks::<3>().0.iter() {
            let source = [
                vertices[tri[0] as usize],
                vertices[tri[1] as usize],
                vertices[tri[2] as usize],
            ];
            let (min, max) = tri_bounds(source.map(|v| v.position));
            for clip_tri in clip {
                let (clip_min, clip_max) = tri_bounds(*clip_tri);
                if clip_min[0] > max[0]
                    || clip_min[1] > max[1]
                    || clip_max[0] < min[0]
                    || clip_max[1] < min[1]
                {
                    continue;
                }
                let mut polygon = source.to_vec();
                for edge in 0..3 {
                    let a = clip_tri[edge];
                    let b = clip_tri[(edge + 1) % 3];
                    polygon = clip_polygon(&polygon, a, b);
                    if polygon.len() < 3 {
                        break;
                    }
                }
                if polygon.len() < 3 {
                    continue;
                }
                let base = self.vertices.len() as u32;
                self.vertices.extend(polygon.iter().copied());
                for k in 1..polygon.len() as u32 - 1 {
                    self.indices.extend([base, base + k, base + k + 1]);
                }
            }
        }
    }

    fn finish(self) -> Option<PathMesh> {
        if self.indices.is_empty() {
            return None;
        }
        let mut min = [f32::INFINITY; 2];
        let mut max = [f32::NEG_INFINITY; 2];
        for vertex in &self.vertices {
            min = [
                min[0].min(vertex.position[0]),
                min[1].min(vertex.position[1]),
            ];
            max = [
                max[0].max(vertex.position[0]),
                max[1].max(vertex.position[1]),
            ];
        }
        Some(PathMesh {
            vertices: self.vertices,
            indices: self.indices,
            bounds: SceneRect {
                x: min[0],
                y: min[1],
                width: max[0] - min[0],
                height: max[1] - min[1],
            },
            gradient: None,
        })
    }
}

/// How far the shadow band may reach in and out at each vertex: `inset` and
/// `blur`, except where offsetting an edge's two ends along their miters
/// would reverse it — the inner ring of a bend tighter than the blur, or the
/// outer ring of a hollow. There the reach stops short of the fold.
fn ring_reach(points: &[[f32; 2]], miters: &[[f32; 2]], inset: f32, blur: f32) -> Vec<(f32, f32)> {
    let n = points.len();
    // Per edge i (points[i] -> points[i + 1]): the largest offset that keeps
    // it pointing the same way, inwards and outwards.
    let limits: Vec<(f32, f32)> = (0..n)
        .map(|i| {
            let j = (i + 1) % n;
            let e = [points[j][0] - points[i][0], points[j][1] - points[i][1]];
            let dm = [miters[j][0] - miters[i][0], miters[j][1] - miters[i][1]];
            let length = e[0] * e[0] + e[1] * e[1];
            let k = dm[0] * e[0] + dm[1] * e[1];
            let keep = 0.9 * length;
            (
                if k > 1e-9 { keep / k } else { f32::INFINITY },
                if k < -1e-9 { keep / -k } else { f32::INFINITY },
            )
        })
        .collect();
    let mut reach: Vec<(f32, f32)> = (0..n)
        .map(|i| {
            let (prev, next) = (limits[(i + n - 1) % n], limits[i]);
            (inset.min(prev.0).min(next.0), blur.min(prev.1).min(next.1))
        })
        .collect();
    // Neighbours may differ by at most half the edge between them. Without
    // this a long edge next to a clamped bend keeps its full reach at the
    // shared vertex and its quad shears across the bend's; a full edge length
    // (what a distance field allows) still lets the shared vertex step past
    // the bend's centre.
    let lengths: Vec<f32> = (0..n)
        .map(|i| {
            let j = (i + 1) % n;
            let e = [points[j][0] - points[i][0], points[j][1] - points[i][1]];
            (e[0] * e[0] + e[1] * e[1]).sqrt()
        })
        .collect();
    for _ in 0..2 {
        for i in 0..n {
            let j = (i + 1) % n;
            reach[j].0 = reach[j].0.min(reach[i].0 + lengths[i] * 0.5);
            reach[j].1 = reach[j].1.min(reach[i].1 + lengths[i] * 0.5);
        }
        for i in (0..n).rev() {
            let j = (i + 1) % n;
            reach[i].0 = reach[i].0.min(reach[j].0 + lengths[i] * 0.5);
            reach[i].1 = reach[i].1.min(reach[j].1 + lengths[i] * 0.5);
        }
    }
    reach
}

fn tri_bounds(tri: [[f32; 2]; 3]) -> ([f32; 2], [f32; 2]) {
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
fn clip_polygon(polygon: &[PathVertex], a: [f32; 2], b: [f32; 2]) -> Vec<PathVertex> {
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

fn lerp_vertex(a: PathVertex, b: PathVertex, t: f32) -> PathVertex {
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

#[cfg(test)]
mod tests {
    use super::*;

    fn rect_path(x: f32, y: f32, width: f32, height: f32) -> PaintPath {
        // Drawn edge by edge, so it stays a path: `PaintPath::rect` would be
        // drawn as a quad and these tests are about the path geometry.
        let mut path = PaintPath::new();
        path.move_to(x, y)
            .line_to(x + width, y)
            .line_to(x + width, y + height)
            .line_to(x, y + height)
            .close();
        path
    }

    fn only_mesh(built: &[BuiltOp]) -> &PathMesh {
        match built {
            [BuiltOp::Mesh(mesh)] => mesh,
            other => panic!("expected one mesh, got {other:?}"),
        }
    }

    /// Coverage the mesh interpolates at `p`, summed over every triangle that
    /// holds it (overlap would show up as more than one).
    fn coverage_at(mesh: &PathMesh, p: [f32; 2]) -> (f32, usize) {
        let mut total = 0.0;
        let mut hits = 0;
        for tri in mesh.indices.as_chunks::<3>().0.iter() {
            let [a, b, c] = [0, 1, 2].map(|k| mesh.vertices[tri[k] as usize]);
            let area = cross(a.position, b.position, c.position);
            if area.abs() < 1e-9 {
                continue;
            }
            let wa = cross(b.position, c.position, p) / area;
            let wb = cross(c.position, a.position, p) / area;
            let wc = 1.0 - wa - wb;
            if wa >= -1e-5 && wb >= -1e-5 && wc >= -1e-5 {
                total += wa * a.coverage + wb * b.coverage + wc * c.coverage;
                hits += 1;
            }
        }
        (total, hits)
    }

    #[test]
    fn a_shadow_ramps_with_distance_to_the_outline_and_never_overlaps() {
        let built = build_ops(&[PaintOp::Shadow {
            path: Arc::new(rect_path(0.0, 0.0, 100.0, 60.0)),
            shadow: ComponentElevation {
                color: [0.0, 0.0, 0.0, 0.5],
                offset_x: 0.0,
                offset_y: 6.0,
                blur_radius: 12.0,
                spread_radius: 0.0,
                inset: false,
            },
        }]);
        let mesh = only_mesh(&built);
        // Below the bottom edge (y = 66 after the offset), and left of the left
        // edge (x = 0), the ramp is the same: 0.5 on the edge, 0 at `blur`.
        for (label, at) in [
            ("bottom", |d: f32| [50.0, 66.0 + d]),
            ("left", |d: f32| [-d, 30.0]),
        ] as [(&str, fn(f32) -> [f32; 2]); 2]
        {
            for d in [-9.0, -4.0, 0.0, 4.0, 9.0] {
                let (coverage, hits) = coverage_at(mesh, at(d));
                assert_eq!(hits, 1, "{label} d={d}: one triangle, no overlap");
                let expected = (12.0 - d) / 24.0;
                assert!(
                    (coverage - expected).abs() < 0.05,
                    "{label} d={d}: coverage {coverage}, expected {expected}"
                );
            }
        }
    }

    #[test]
    fn a_solid_rounded_rectangle_fills_and_strokes_as_a_quad() {
        let mut card = PaintPath::new();
        card.rounded_rect(
            LayoutBox {
                x: 10.0,
                y: 10.0,
                width: 80.0,
                height: 40.0,
            },
            [6.0, 6.0, 0.0, 0.0],
        );
        let card = Arc::new(card);
        let solid = ResolvedPaint::Solid([1.0, 0.0, 0.0, 1.0]);
        let stroke = |join| {
            let mut stroke = StrokeStyle::new(2.0);
            stroke.join = join;
            stroke
        };
        let built = build_ops(&[
            PaintOp::FillPath {
                path: Arc::clone(&card),
                paint: solid.clone(),
            },
            PaintOp::StrokePath {
                path: Arc::clone(&card),
                stroke: stroke(LineJoin::Miter),
                paint: solid.clone(),
            },
            PaintOp::StrokePath {
                path: Arc::clone(&card),
                stroke: stroke(LineJoin::Round),
                paint: solid.clone(),
            },
        ]);
        let [
            BuiltOp::Quad {
                rect: fill_rect,
                radii: fill_radii,
                fill: Some(_),
                border: None,
                ..
            },
            BuiltOp::Quad {
                rect: miter_rect,
                radii: miter_radii,
                fill: None,
                border: Some((_, 2.0)),
                ..
            },
            BuiltOp::Quad {
                radii: round_radii, ..
            },
        ] = &built[..]
        else {
            panic!("{built:?}");
        };
        assert_eq!((fill_rect.x, fill_rect.width), (10.0, 80.0));
        assert_eq!(*fill_radii, [6.0, 6.0, 0.0, 0.0]);
        // Half the width either side of the outline; arcs grow by half, a
        // square corner stays square under a miter and rounds under a round
        // join.
        assert_eq!((miter_rect.x, miter_rect.width), (9.0, 82.0));
        assert_eq!(*miter_radii, [7.0, 7.0, 0.0, 0.0]);
        assert_eq!(*round_radii, [7.0, 7.0, 1.0, 1.0]);

        // What a quad border cannot draw stays a path: a dash, a bevelled
        // square corner, a gradient, anything not a rectangle.
        let mut dashed = StrokeStyle::new(2.0).dash(vec![4.0, 2.0], 0.0);
        dashed.join = LineJoin::Miter;
        let mut circle = PaintPath::new();
        circle.ellipse(LayoutBox {
            x: 0.0,
            y: 0.0,
            width: 10.0,
            height: 10.0,
        });
        let built = build_ops(&[
            PaintOp::StrokePath {
                path: Arc::clone(&card),
                stroke: dashed,
                paint: solid.clone(),
            },
            PaintOp::StrokePath {
                path: Arc::clone(&card),
                stroke: stroke(LineJoin::Bevel),
                paint: solid.clone(),
            },
            PaintOp::FillPath {
                path: Arc::new(circle),
                paint: solid,
            },
        ]);
        assert!(
            built.iter().all(|op| matches!(op, BuiltOp::Mesh(_))),
            "{built:?}"
        );
    }

    #[test]
    fn non_finite_or_far_off_input_draws_nothing_instead_of_hanging() {
        let mut far = PaintPath::new();
        far.move_to(0.0, 0.0)
            .cubic_to(1e30, 0.0, 0.0, 1e30, 1e30, 1e30)
            .close();
        let mut nan = PaintPath::new();
        nan.move_to(0.0, 0.0)
            .line_to(f32::NAN, 5.0)
            .line_to(5.0, 5.0)
            .close();
        let square = Arc::new(rect_path(0.0, 0.0, 10.0, 10.0));
        let shadow = |blur_radius: f32| ComponentElevation {
            color: [0.0, 0.0, 0.0, 1.0],
            offset_x: 0.0,
            offset_y: 0.0,
            blur_radius,
            spread_radius: 0.0,
            inset: false,
        };
        let built = build_ops(&[
            PaintOp::FillPath {
                path: Arc::new(far.clone()),
                paint: ResolvedPaint::Solid([1.0; 4]),
            },
            PaintOp::FillPath {
                path: Arc::new(nan),
                paint: ResolvedPaint::Solid([1.0; 4]),
            },
            PaintOp::StrokePath {
                path: Arc::new(far),
                stroke: StrokeStyle::new(2.0),
                paint: ResolvedPaint::Solid([1.0; 4]),
            },
            PaintOp::StrokePath {
                path: Arc::clone(&square),
                stroke: StrokeStyle::new(f32::INFINITY),
                paint: ResolvedPaint::Solid([1.0; 4]),
            },
            PaintOp::Shadow {
                path: Arc::clone(&square),
                shadow: shadow(f32::INFINITY),
            },
            // A huge but finite transform on a small path.
            PaintOp::SetTransform([1e12, 0.0, 0.0, 1e12, 0.0, 0.0]),
            PaintOp::FillPath {
                path: square,
                paint: ResolvedPaint::Solid([1.0; 4]),
            },
        ]);
        assert!(built.is_empty(), "{} ops", built.len());
    }

    #[test]
    fn an_inset_shadow_covers_every_point_of_a_corner_once() {
        let built = build_ops(&[PaintOp::Shadow {
            path: Arc::new(rect_path(0.0, 0.0, 100.0, 60.0)),
            shadow: ComponentElevation {
                color: [0.0, 0.0, 0.0, 1.0],
                offset_x: 20.0,
                offset_y: 20.0,
                blur_radius: 8.0,
                spread_radius: 0.0,
                inset: true,
            },
        }]);
        let mesh = only_mesh(&built);
        // The offset puts the band's mitred corner (12, 12) inside the
        // outline, where the solid region past it has to meet it without
        // overlap. Sample at offsets no triangle edge runs through: a point
        // on a shared edge counts for both triangles.
        for y in 0..30 {
            for x in 0..30 {
                let p = [
                    x as f32 + std::f32::consts::FRAC_1_SQRT_2,
                    y as f32 + std::f32::consts::FRAC_1_PI,
                ];
                let (coverage, hits) = coverage_at(mesh, p);
                assert!(
                    hits <= 1 && coverage <= 1.0001,
                    "coverage {coverage} at {p:?} ({hits} triangles)"
                );
            }
        }
    }

    #[test]
    fn a_shape_thinner_than_its_blur_casts_a_faint_shadow_without_overlap() {
        let circle = |r: f32| LayoutBox {
            x: 50.0 - r,
            y: 50.0 - r,
            width: r * 2.0,
            height: r * 2.0,
        };
        let mut ring = PaintPath::new();
        ring.ellipse(circle(50.0)).ellipse(circle(48.0));
        let ring = ring.with_fill_rule(FillRule::EvenOdd);
        let shadow = ComponentElevation {
            color: [0.0, 0.0, 0.0, 1.0],
            offset_x: 0.0,
            offset_y: 0.0,
            blur_radius: 10.0,
            spread_radius: 0.0,
            inset: false,
        };
        let built = build_ops(&[PaintOp::Shadow {
            path: Arc::new(ring),
            shadow,
        }]);
        let mesh = only_mesh(&built);
        // A 2px ring under a 10px blur peaks near erf(4 / (√2·10)) ≈ 0.31
        // (at most twice the true width), not at full strength.
        let alpha = mesh.vertices[0].color[3];
        assert!(alpha < 0.4, "peak alpha {alpha}");
        for r in [40.0, 45.0, 49.0, 53.0, 58.0] {
            let (coverage, hits) = coverage_at(mesh, [50.0 + r, 50.25]);
            assert!(hits <= 1, "r={r}: {hits} triangles overlap");
            assert!(coverage * alpha < 0.4, "r={r}: {}", coverage * alpha);
        }
        // A disc well wider than its blur keeps its full strength.
        let mut disc = PaintPath::new();
        disc.ellipse(circle(50.0));
        let built = build_ops(&[PaintOp::Shadow {
            path: Arc::new(disc),
            shadow,
        }]);
        assert!(only_mesh(&built).vertices[0].color[3] > 0.99);
    }

    #[test]
    fn a_corner_tighter_than_the_blur_does_not_fold_the_band() {
        let mut path = PaintPath::new();
        path.rounded_rect(
            LayoutBox {
                x: 0.0,
                y: 0.0,
                width: 100.0,
                height: 60.0,
            },
            [6.0; 4],
        );
        let built = build_ops(&[PaintOp::Shadow {
            path: Arc::new(path),
            shadow: ComponentElevation {
                color: [0.0, 0.0, 0.0, 0.5],
                offset_x: 0.0,
                offset_y: 0.0,
                blur_radius: 16.0,
                spread_radius: 0.0,
                inset: false,
            },
        }]);
        let mesh = only_mesh(&built);
        // Every point in a grid around the top-left corner is covered once at
        // most, and coverage never rises going outwards along the diagonal.
        for y in -14..14 {
            for x in -14..14 {
                let p = [x as f32 + 0.25, y as f32 + 0.25];
                assert!(coverage_at(mesh, p).1 <= 1, "overlap at {p:?}");
            }
        }
        let mut last = f32::INFINITY;
        for step in 0..30 {
            let t = 14.0 - step as f32;
            let (coverage, _) = coverage_at(mesh, [t, t]);
            assert!(coverage <= last + 1e-4, "coverage rose at {t}");
            last = coverage;
        }
    }

    #[test]
    fn a_shadow_band_spans_blur_either_side_of_the_offset_outline() {
        let built = build_ops(&[PaintOp::Shadow {
            path: Arc::new(rect_path(0.0, 0.0, 100.0, 60.0)),
            shadow: ComponentElevation {
                color: [0.0, 0.0, 0.0, 0.5],
                offset_x: 0.0,
                offset_y: 6.0,
                blur_radius: 12.0,
                spread_radius: 0.0,
                inset: false,
            },
        }]);
        let mesh = only_mesh(&built);
        let b = mesh.bounds;
        assert!((b.x + 12.0).abs() < 0.6, "{b:?}");
        assert!((b.y - (6.0 - 12.0)).abs() < 0.6, "{b:?}");
        assert!((b.width - 124.0).abs() < 1.2, "{b:?}");
        assert!((b.height - 84.0).abs() < 1.2, "{b:?}");
    }

    #[test]
    fn dashes_follow_the_pattern_and_its_offset() {
        let line: Contour = vec![[0.0, 0.0], [40.0, 0.0]];
        let spans = |pieces: Vec<Contour>| -> Vec<(f64, f64)> {
            pieces
                .iter()
                .map(|piece| (piece[0][0], piece[piece.len() - 1][0]))
                .collect()
        };
        assert_eq!(
            spans(dashes(&line, false, &[10.0, 5.0], 0.0)),
            [(0.0, 10.0), (15.0, 25.0), (30.0, 40.0)]
        );
        assert_eq!(
            spans(dashes(&line, false, &[10.0, 5.0], 12.0)),
            [(3.0, 13.0), (18.0, 28.0), (33.0, 40.0)]
        );
        // A dash runs round a corner as one piece.
        let corner: Contour = vec![[0.0, 0.0], [4.0, 0.0], [4.0, 20.0]];
        let pieces = dashes(&corner, false, &[10.0, 10.0], 0.0);
        assert_eq!(pieces[0], vec![[0.0, 0.0], [4.0, 0.0], [4.0, 6.0]]);
        assert_eq!(dash_pattern(&[3.0]), Some(vec![3.0, 3.0]));
        assert_eq!(dash_pattern(&[3.0, -1.0]), None);
        assert_eq!(dash_pattern(&[0.0, 0.0]), None);
    }

    #[test]
    fn a_dashed_stroke_leaves_its_gaps_empty() {
        let mut line = PaintPath::new();
        line.move_to(0.0, 10.0).line_to(40.0, 10.0);
        let built = build_ops(&[PaintOp::StrokePath {
            path: Arc::new(line),
            paint: ResolvedPaint::Solid([1.0; 4]),
            stroke: StrokeStyle::new(4.0).dash(vec![10.0, 5.0], 0.0),
        }]);
        let mesh = only_mesh(&built);
        // Off the triangles' shared diagonals, so a hit counts once.
        assert_eq!(coverage_at(mesh, [5.3, 10.7]).1, 1);
        assert_eq!(coverage_at(mesh, [12.5, 10.7]).1, 0, "the first gap");
        assert_eq!(coverage_at(mesh, [20.3, 10.7]).1, 1);
    }

    #[test]
    fn geometry_is_laid_out_under_the_local_transform() {
        let scale = [2.0, 0.0, 0.0, 2.0, 10.0, 0.0];
        let mut line = PaintPath::new();
        line.move_to(0.0, 10.0).line_to(20.0, 10.0);
        let built = build_ops(&[
            PaintOp::SetTransform(scale),
            PaintOp::FillPath {
                path: Arc::new(rect_path(0.0, 0.0, 5.0, 5.0)),
                paint: ResolvedPaint::Solid([1.0; 4]),
            },
            PaintOp::StrokePath {
                path: Arc::new(line),
                paint: ResolvedPaint::Solid([1.0; 4]),
                stroke: StrokeStyle::new(2.0),
            },
        ]);
        let [BuiltOp::Mesh(fill), BuiltOp::Mesh(stroke)] = &built[..] else {
            panic!("{built:?}");
        };
        let b = fill.bounds;
        assert_eq!((b.x, b.y, b.width, b.height), (10.0, 0.0, 10.0, 10.0));
        // The stroke is laid out first and scaled with the path: 2px becomes 4.
        let b = stroke.bounds;
        assert!(
            (b.height - 4.0).abs() < 1e-3 && (b.y - 18.0).abs() < 1e-3,
            "{b:?}"
        );
        assert!(
            (b.x - 10.0).abs() < 1e-3 && (b.width - 40.0).abs() < 1e-3,
            "{b:?}"
        );
    }

    #[test]
    fn a_gradient_mesh_remembers_where_its_vertices_came_from() {
        let gradient = Arc::new(ResolvedGradient {
            shape: nana_ui_runtime::GradientShape::Linear {
                start: [0.0, 0.0],
                end: [10.0, 0.0],
            },
            stops: vec![(0.0, [1.0, 0.0, 0.0, 1.0]), (1.0, [0.0, 0.0, 1.0, 1.0])],
            extend: Default::default(),
        });
        let built = build_ops(&[
            PaintOp::SetTransform([1.0, 0.0, 0.0, 1.0, 30.0, 40.0]),
            PaintOp::FillPath {
                path: Arc::new(rect_path(0.0, 0.0, 10.0, 10.0)),
                paint: ResolvedPaint::Gradient(Arc::clone(&gradient)),
            },
        ]);
        let mesh = only_mesh(&built);
        assert!(Arc::ptr_eq(mesh.gradient.as_ref().unwrap(), &gradient));
        for vertex in &mesh.vertices {
            assert_eq!(
                vertex.paint_pos,
                [vertex.position[0] - 30.0, vertex.position[1] - 40.0]
            );
            assert_eq!(vertex.color, [1.0, 1.0, 1.0, 1.0], "only a tint");
        }
    }

    fn text_op(paint: ResolvedPaint) -> PaintOp {
        PaintOp::Text {
            rect: LayoutBox {
                x: 0.0,
                y: 0.0,
                width: 40.0,
                height: 16.0,
            },
            content: Arc::from("label"),
            paint,
            size: 13.0,
            weight: None,
            italic: false,
            line_height: None,
            wrap: false,
            max_lines: None,
            horizontal: Default::default(),
            vertical: Default::default(),
        }
    }

    fn circle(cx: f32, cy: f32, r: f32) -> PaintPath {
        let mut path = PaintPath::new();
        path.ellipse(LayoutBox {
            x: cx - r,
            y: cy - r,
            width: r * 2.0,
            height: r * 2.0,
        });
        path
    }

    fn kinds(built: &[BuiltOp]) -> Vec<&'static str> {
        built
            .iter()
            .map(|op| match op {
                BuiltOp::Mesh(_) => "mesh",
                BuiltOp::Quad { .. } => "quad",
                BuiltOp::Image { .. } => "image",
                BuiltOp::Text { .. } => "text",
                BuiltOp::Icon { .. } => "icon",
                BuiltOp::Default { .. } => "default",
                BuiltOp::LayerBegin { .. } => "begin",
                BuiltOp::LayerEnd { mask: None } => "end",
                BuiltOp::LayerEnd {
                    mask: Some((_, LayerMaskMode::Erase)),
                } => "end-erase",
                BuiltOp::LayerEnd {
                    mask: Some((_, LayerMaskMode::Tint)),
                } => "end-tint",
            })
            .collect()
    }

    #[test]
    fn opacity_scales_each_op_it_governs() {
        let built = build_ops(&[
            PaintOp::SetOpacity(0.5),
            PaintOp::FillPath {
                path: Arc::new(rect_path(0.0, 0.0, 10.0, 10.0)),
                paint: ResolvedPaint::Solid([1.0, 0.0, 0.0, 0.8]),
            },
            PaintOp::RoundedRect {
                rect: LayoutBox {
                    x: 0.0,
                    y: 0.0,
                    width: 10.0,
                    height: 10.0,
                },
                radii: [2.0; 4],
                fill: Some(ResolvedPaint::Solid([1.0; 4])),
                border: None,
                shadow: None,
            },
            text_op(ResolvedPaint::Solid([1.0; 4])),
            PaintOp::DrawDefault,
        ]);
        let [
            BuiltOp::Mesh(mesh),
            BuiltOp::Quad { fill, .. },
            BuiltOp::Text { color, .. },
            BuiltOp::Default { opacity, .. },
        ] = &built[..]
        else {
            panic!("{built:?}");
        };
        assert!(
            mesh.vertices
                .iter()
                .all(|v| (v.color[3] - 0.4).abs() < 1e-6)
        );
        assert_eq!(fill.unwrap()[3], 0.5);
        assert_eq!(color[3], 0.5);
        assert_eq!(*opacity, 0.5);
    }

    #[test]
    fn a_gradient_on_glyphs_tints_them_in_a_layer() {
        let gradient = Arc::new(ResolvedGradient {
            shape: nana_ui_runtime::GradientShape::Linear {
                start: [0.0, 0.0],
                end: [40.0, 0.0],
            },
            stops: vec![(0.0, [1.0, 0.0, 0.0, 1.0]), (1.0, [0.0, 0.0, 1.0, 1.0])],
            extend: Default::default(),
        });
        let built = build_ops(&[text_op(ResolvedPaint::Gradient(gradient))]);
        assert_eq!(kinds(&built), ["begin", "text", "end-tint"]);
        let BuiltOp::Text { color, .. } = &built[1] else {
            unreachable!()
        };
        assert_eq!(*color, [1.0; 4], "the glyphs are only coverage");
    }

    #[test]
    fn a_clip_the_gpu_cannot_express_masks_text_in_a_layer() {
        let clipped = |clip: PaintPath| {
            build_ops(&[
                PaintOp::PushClip {
                    path: Arc::new(clip),
                },
                text_op(ResolvedPaint::Solid([1.0; 4])),
                PaintOp::DrawDefault,
                PaintOp::PopClip,
            ])
        };
        // A rectangle is exact on the GPU: no layer.
        let exact = clipped(rect_path(0.0, 0.0, 20.0, 20.0));
        assert_eq!(kinds(&exact), ["text", "default"]);
        let BuiltOp::Text { clips, .. } = &exact[0] else {
            unreachable!()
        };
        assert_eq!(clips.len(), 1);
        // A circle is not: the text is drawn in a layer cut down to it.
        let masked = clipped(circle(20.0, 20.0, 15.0));
        assert_eq!(
            kinds(&masked),
            [
                "begin",
                "text",
                "end-erase",
                "begin",
                "default",
                "end-erase"
            ]
        );
        let BuiltOp::LayerBegin {
            clip: Some(area), ..
        } = &masked[0]
        else {
            unreachable!()
        };
        assert!(
            (area.x - 4.0).abs() < 0.1 && (area.width - 32.0).abs() < 0.2,
            "{area:?}"
        );
        let BuiltOp::LayerEnd {
            mask: Some((erase, _)),
        } = &masked[2]
        else {
            unreachable!()
        };
        // The erase covers the corner of the area and leaves the centre.
        assert_eq!(coverage_at(erase, [5.0, 5.0]).1, 1);
        assert_eq!(coverage_at(erase, [20.3, 20.7]).1, 0);
    }

    #[test]
    fn an_image_fills_a_path_through_a_masked_layer() {
        let built = build_ops(&[PaintOp::FillImage {
            path: Arc::new(circle(20.0, 20.0, 10.0)),
            rect: LayoutBox {
                x: 0.0,
                y: 0.0,
                width: 40.0,
                height: 40.0,
            },
            source: Arc::from("data:image/png;base64,"),
            fit: ImageFit::Cover,
        }]);
        assert_eq!(kinds(&built), ["begin", "image", "end-erase"]);
    }

    #[test]
    fn an_inset_shadow_stays_inside_and_darkens_toward_the_edge() {
        let built = build_ops(&[PaintOp::Shadow {
            path: Arc::new(rect_path(0.0, 0.0, 100.0, 60.0)),
            shadow: ComponentElevation {
                color: [0.0, 0.0, 0.0, 0.5],
                offset_x: 0.0,
                offset_y: 0.0,
                blur_radius: 8.0,
                spread_radius: 0.0,
                inset: true,
            },
        }]);
        let mesh = only_mesh(&built);
        let b = mesh.bounds;
        assert!(
            b.x >= -1e-3 && b.y >= -1e-3 && b.x + b.width <= 100.001,
            "{b:?}"
        );
        let at = |p| coverage_at(mesh, p).0;
        assert!(at([1.3, 30.7]) > at([6.3, 30.7]), "darker at the edge");
        assert!(at([50.3, 30.7]) < 0.05, "clear in the middle");
    }
}
