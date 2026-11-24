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
//!
//! A new recording of the node (content edited live, a canvas that redraws)
//! starts from the meshes the last one built: each mesh-producing op is keyed
//! by everything its geometry depends on (see [`MeshReuse`]), and an op whose
//! key the last recording built keeps that mesh, recoloured if only its paint
//! changed. Only the ops that really changed are built again.

use std::cell::OnceCell;
use std::sync::OnceLock;

use nana_ui_runtime::{
    AFFINE_IDENTITY, Affine, BlendMode, FillRule, ImageFit, LineCap, LineJoin, PaintOp, PaintPath,
    PaintPhase, PaintRecording, PathVerb, ResolvedGradient, ResolvedPaint, StrokeStyle,
};

use super::*;

pub(super) type Point = [f64; 2];
pub(super) type Contour = Vec<Point>;

mod effects;
mod stroke;
mod tessellate;
#[cfg(test)]
mod tests;

use effects::*;
use stroke::*;
use tessellate::*;
pub(super) type Shapes = Vec<Vec<Contour>>;

/// Curve flattening tolerance in logical px. A 2x display sees 0.1 physical
/// px of error at most.
pub(super) const TOLERANCE: f32 = 0.05;
/// Longest a miter extrusion may get, in units of the offset. Keeps the AA
/// fringe and shadow band of a spike from shooting across the node.
pub(super) const MITER_LIMIT: f32 = 4.0;

/// Triangles a node's painter produced, in node-local logical px.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct PathMesh {
    pub vertices: Vec<PathVertex>,
    pub indices: Vec<u32>,
    /// Local bounds of every vertex, before the AA fringe (which reaches half
    /// a physical px further out).
    pub bounds: SceneRect,
    /// Colour every fragment from this gradient, evaluated at the vertex's
    /// [`PathVertex::paint_pos`]; the vertex colour then only tints it
    /// (coverage and opacity). `None`: the vertex colour is the colour.
    pub gradient: Option<Arc<ResolvedGradient>>,
    /// Explicit authoring-space solid, if the mesh was made from one. The
    /// GPU uses its linear scRGB channels directly instead of decoding the
    /// compatibility sRGB vertex colour a second time.
    pub paint_color: Option<nana_ui_core::PaintColor>,
}

/// One tessellated vertex.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PathVertex {
    pub position: [f32; 2],
    /// The miter along which this vertex moves half a physical pixel, onto its
    /// anti-aliasing fringe's outer ring (outward) or inner ring (inward).
    /// Zero for every vertex off a fringe.
    pub extrude: [f32; 2],
    /// Linear coverage in `0..=1`. Across an AA fringe (a non-zero
    /// [`PathVertex::extrude`]) the painter keeps it linear, as every other
    /// edge ramps; across a blurred shadow's band of `±2σ` it shapes it with a
    /// smoothstep, a close fit of a Gaussian edge.
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
pub(super) enum LocalClip {
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
pub(super) enum BuiltOp {
    Mesh(Arc<PathMesh>),
    Quad {
        rect: SceneRect,
        radii: [f32; 4],
        fill: Option<[f32; 4]>,
        fill_color_space: Option<nana_ui_core::PaintColor>,
        border: Option<([f32; 4], f32)>,
        border_color_space: Option<nana_ui_core::PaintColor>,
        shadow: Option<ComponentElevation>,
        transform: Affine,
    },
    /// Solid quads painted one after another with nothing between them, the
    /// same corners and transform, and no border, shadow or authoring color:
    /// one primitive however many, each with its own box and color.
    QuadRun {
        items: Vec<(SceneRect, [f32; 4])>,
        radii: [f32; 4],
        transform: Affine,
    },
    Image {
        rect: SceneRect,
        source: Arc<str>,
        fit: ImageFit,
        sampling: nana_ui_core::ImageSampling,
        radii: [f32; 4],
        opacity: f32,
        clips: Arc<[LocalClip]>,
        transform: Affine,
    },
    Text {
        rect: SceneRect,
        text: Arc<TextStyle>,
        color: [f32; 4],
        paint_color: Option<nana_ui_core::PaintColor>,
        clips: Arc<[LocalClip]>,
        transform: Affine,
    },
    Icon {
        rect: SceneRect,
        icon: Icon,
        color: [f32; 4],
        paint_color: Option<nana_ui_core::PaintColor>,
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
pub(super) struct TextStyle {
    content: Arc<str>,
    size: f32,
    weight: Option<u16>,
    family: Option<Arc<str>>,
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
    /// The node's `mask-image` (see [`node_mask`]). Whether there is one
    /// decides how the ops are built: see [`build_ops`].
    node_mask: Option<Arc<NodeMask>>,
    /// Behind one pointer: the scene keeps a `BuiltPaint` per painted node
    /// in tables whose every slot is its size.
    ops: Arc<BuiltOps>,
}

#[derive(Debug)]
pub(super) struct BuiltOps {
    behind: Box<[BuiltOp]>,
    over: Box<[BuiltOp]>,
    /// The meshes and clips built for `behind` and `over`, by key, for the
    /// node's next recording to start from.
    meshes: MeshCache,
}

impl BuiltPaint {
    /// Build `recording`'s geometry, reusing what `previous` built for any op
    /// that did not change.
    fn build(
        recording: &Arc<PaintRecording>,
        node_mask: Option<Arc<NodeMask>>,
        over: bool,
        previous: Vec<&MeshCache>,
    ) -> Self {
        let masked = node_mask.is_some();
        let mut reuse = MeshReuse::new(previous);
        let behind = build_ops(&recording.behind_children, masked, &mut reuse).into();
        let over = if over {
            build_ops(&recording.over_children, masked, &mut reuse).into()
        } else {
            Box::from([])
        };
        Self {
            recording: Arc::clone(recording),
            node_mask,
            ops: Arc::new(BuiltOps {
                behind,
                over,
                meshes: reuse.finish(),
            }),
        }
    }
}

/// The slot namespaces a painted node's own primitives live in. Their paint
/// order is decided by [`custom_paint_stack`], which reads the namespace.
pub(super) const PAINT_BEHIND_PRE: u32 = 0xFFFF_FFF0;
pub(super) const PAINT_BEHIND_POST: u32 = 0xFFFF_FFF1;
pub(super) const PAINT_OVER_PRE: u32 = 0xFFFF_FFF2;
pub(super) const PAINT_OVER_POST: u32 = 0xFFFF_FFF3;
/// Namespace for vector recordings emitted through [`nana_ui_runtime::WidgetPaintOp::Vector`].
pub(super) const WIDGET_VECTOR_PAINT: u32 = 19;
pub(super) const WIDGET_VECTOR_BLOCK: usize = 1 << 18;

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
    match namespace {
        PAINT_BEHIND_PRE => None,
        PAINT_BEHIND_POST => Some((i32::MIN, 0)),
        PAINT_OVER_PRE => Some((i32::MAX, usize::MAX - 1)),
        PAINT_OVER_POST => Some((i32::MAX, usize::MAX)),
        // The node's built-in primitives: wherever `draw_default()` put them.
        // Asked only for these — every painted primitive orders through here,
        // and finding the call scans the whole recording.
        _ if recording.default_phase() == Some(PaintPhase::OverChildren) => {
            Some((i32::MAX, usize::MAX))
        }
        _ => Some((i32::MIN, 0)),
    }
}

/// The node's `mask-image` as its painter's output takes it: a gradient
/// over a box with area. A `url()` mask is counted and not applied — the
/// painter's output draws unmasked — and neither is a mask over an empty box.
pub(super) fn node_mask(node: &ExtractedNode) -> Option<NodeMask> {
    let image = node.source_style.layout.paint.mask.as_ref()?;
    let size = [node.layout.width, node.layout.height];
    if !size.iter().all(|axis| axis.is_finite() && *axis > 0.0) {
        return None;
    }
    match image {
        nana_ui_core::MaskImage::Gradient(gradient) => Some(NodeMask {
            gradient: gradient.clone(),
            size,
        }),
        nana_ui_core::MaskImage::Url(_) => {
            nana_diagnostics::metric!(nana_diagnostics::framework::runtime::PAINT_MASK_UNSUPPORTED);
            None
        }
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
        node_mask: Option<Arc<NodeMask>>,
    ) -> BuiltPaint {
        let masked = node_mask.is_some();
        match self.custom_paint.get(&id) {
            Some(built)
                if Arc::ptr_eq(&built.recording, recording)
                    && built.node_mask.is_some() == masked =>
            {
                BuiltPaint {
                    node_mask,
                    ..built.clone()
                }
            }
            held => {
                let previous = held.map(|held| &held.ops.meshes).into_iter().collect();
                let built = BuiltPaint::build(recording, node_mask, true, previous);
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
        self.emit_built_ops(
            node,
            built,
            PaintPlacement {
                origin: {
                    let record = Self::record_box(node);
                    [record.x, record.y]
                },
                size: [node.layout.width, node.layout.height],
                phases: [
                    (PaintPhaseOps::Behind, PAINT_BEHIND_PRE, PAINT_BEHIND_POST),
                    (PaintPhaseOps::Over, PAINT_OVER_PRE, PAINT_OVER_POST),
                ],
                adjust_default: true,
                index_base: 0,
            },
            transform,
            clips,
            opacity,
            node_order,
        );
    }

    /// Emit vector recordings emitted through [`nana_ui_runtime::WidgetPaintOp::Vector`],
    /// reusing built meshes across frames while the recording `Arc` is unchanged.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn emit_widget_vector_paint(
        &mut self,
        node: &ExtractedNode,
        vectors: &[(u64, nana_ui_runtime::LayoutBox, Arc<PaintRecording>)],
        transform: AffineTransform,
        clips: &Arc<[ClipRegion]>,
        opacity: f32,
        node_order: usize,
    ) {
        let held = self.widget_vector_paint.get(&node.id).cloned();
        let mut built = Vec::with_capacity(vectors.len());
        let mut places = Vec::with_capacity(vectors.len());
        for (slot, bounds, recording) in vectors {
            let reused = held
                .as_deref()
                .and_then(|held| held.iter().find(|b| Arc::ptr_eq(&b.recording, recording)))
                .cloned();
            built.push(reused.unwrap_or_else(|| {
                let previous = held
                    .iter()
                    .flat_map(|held| held.iter())
                    .map(|b| &b.ops.meshes);
                BuiltPaint::build(recording, None, false, previous.collect())
            }));
            places.push((
                *slot,
                [bounds.x, bounds.y],
                [bounds.width.max(0.0), bounds.height.max(0.0)],
            ));
        }
        let built: Arc<[BuiltPaint]> = built.into();
        self.widget_vector_paint.insert(node.id, Arc::clone(&built));
        for (index, (paint, (slot, origin, size))) in built.iter().zip(places).enumerate() {
            let namespace = if (1..=255).contains(&slot) {
                nana_ui_runtime::WIDGET_VECTOR_LAYER_BASE | (slot as u32)
            } else {
                WIDGET_VECTOR_PAINT
            };
            self.emit_built_ops(
                node,
                paint,
                PaintPlacement {
                    origin,
                    size,
                    phases: [
                        (PaintPhaseOps::Behind, namespace, namespace),
                        (PaintPhaseOps::None, namespace, namespace),
                    ],
                    adjust_default: false,
                    index_base: index * WIDGET_VECTOR_BLOCK,
                },
                transform,
                clips,
                opacity,
                node_order,
            );
        }
    }

    /// Emit a recording's built ops at `placement` for `node`.
    #[allow(clippy::too_many_arguments)]
    fn emit_built_ops(
        &mut self,
        node: &ExtractedNode,
        built: &BuiltPaint,
        placement: PaintPlacement,
        transform: AffineTransform,
        clips: &Arc<[ClipRegion]>,
        opacity: f32,
        node_order: usize,
    ) {
        let node_mask = built.node_mask.as_ref();
        let origin = placement.origin;
        // A painter's local transform turns about the recording's corner on
        // the page, where the node's geometry is drawn: one that turns
        // anything is built again when the node moves.
        let pivot = {
            let [x, y] = self.paint_origin(node);
            [origin[0] + x, origin[1] + y]
        };
        let mut bound = false;
        let [width, height] = placement.size;
        for (phase, pre, post) in placement.phases {
            let ops: &[BuiltOp] = match phase {
                PaintPhaseOps::Behind => &built.ops.behind,
                PaintPhaseOps::Over => &built.ops.over,
                PaintPhaseOps::None => &[],
            };
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
                let under = |local: &Affine| transform.then(about_origin(*local, pivot));
                let mut primitive_opacity = opacity;
                let (bounds, kind, primitive_clips, primitive_transform) = match op {
                    BuiltOp::Default {
                        clips: local,
                        transform: local_transform,
                        opacity: op_opacity,
                    } => {
                        namespace = post;
                        if !placement.adjust_default {
                            continue;
                        }
                        let extra: Vec<ClipRegion> = local
                            .iter()
                            .map(|clip| clip_region(clip, origin, transform))
                            .collect();
                        let moved = (*local_transform != AFFINE_IDENTITY)
                            .then(|| about_origin(*local_transform, pivot));
                        bound |= moved.is_some();
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
                                width,
                                height,
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
                                width,
                                height,
                            },
                            |(mesh, _)| offset_rect(mesh.bounds, origin),
                        ),
                        ScenePrimitiveKind::LayerEnd {
                            mask: mask.as_ref().map(|(mesh, mode)| LayerMask {
                                mesh: Arc::clone(mesh),
                                origin,
                                mode: *mode,
                                node_mask: (*mode == LayerMaskMode::Keep)
                                    .then(|| node_mask.cloned())
                                    .flatten(),
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
                            node_mask: node_mask.cloned(),
                        },
                        Arc::clone(clips),
                        transform,
                    ),
                    BuiltOp::Quad {
                        rect,
                        radii,
                        fill,
                        fill_color_space,
                        border,
                        border_color_space,
                        shadow,
                        transform: local_transform,
                    } => (
                        offset_rect(*rect, origin),
                        ScenePrimitiveKind::Quad {
                            background: super::slot_color(*fill_color_space, *fill),
                            border_color: super::slot_color(
                                *border_color_space,
                                border.map(|(color, _)| color),
                            ),
                            border_width: border.map_or(0.0, |(_, width)| width),
                            corner_radius: *radii,
                            shadow: *shadow,
                            surface: Box::default(),
                        },
                        Arc::clone(clips),
                        under(local_transform),
                    ),
                    BuiltOp::QuadRun {
                        items,
                        radii,
                        transform: local_transform,
                    } => {
                        let rects: Vec<SceneRect> = items
                            .iter()
                            .map(|(rect, _)| offset_rect(*rect, origin))
                            .collect();
                        (
                            union_rect(&rects),
                            ScenePrimitiveKind::QuadColorBatch {
                                bounds: rects,
                                colors: items.iter().map(|(_, color)| *color).collect(),
                                border_color: None,
                                border_width: 0.0,
                                corner_radius: *radii,
                            },
                            Arc::clone(clips),
                            under(local_transform),
                        )
                    }
                    BuiltOp::Image {
                        rect,
                        source,
                        fit,
                        sampling,
                        radii,
                        opacity: op_opacity,
                        clips: local,
                        transform: local_transform,
                    } => {
                        primitive_opacity *= op_opacity;
                        (
                            offset_rect(*rect, origin),
                            image_quad(source, *fit, *sampling, *radii),
                            local_clips(local),
                            under(local_transform),
                        )
                    }
                    BuiltOp::Text {
                        rect,
                        text,
                        color,
                        paint_color,
                        clips: local,
                        transform: local_transform,
                    } => (
                        offset_rect(*rect, origin),
                        custom_text(node, text, *color, *paint_color),
                        local_clips(local),
                        under(local_transform),
                    ),
                    BuiltOp::Icon {
                        rect,
                        icon,
                        color,
                        paint_color,
                        clips: local,
                        transform: local_transform,
                    } => (
                        offset_rect(*rect, origin),
                        ScenePrimitiveKind::Icon {
                            icon: *icon,
                            color: super::slot_color(*paint_color, Some(*color)),
                        },
                        local_clips(local),
                        under(local_transform),
                    ),
                };
                bound |= primitive_transform != transform;
                self.insert_primitive(ScenePrimitive {
                    id: PrimitiveId {
                        node: node.id,
                        slot: collection_slot(namespace, placement.index_base + index),
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
        if bound {
            self.origin_bound.insert(node.id);
        }
    }
}

/// Which of a recording's built phases a placement emits.
#[derive(Clone, Copy)]
pub(super) enum PaintPhaseOps {
    Behind,
    Over,
    None,
}

/// Where a recording's ops land: the box they were recorded in, which
/// phases go out under which slot namespaces (before and after
/// `draw_default()`), and whether `draw_default()` re-states the built-ins.
pub(super) struct PaintPlacement {
    origin: [f32; 2],
    size: [f32; 2],
    phases: [(PaintPhaseOps, u32, u32); 2],
    adjust_default: bool,
    /// Added to every op's slot index, so several recordings can share one
    /// namespace on one node.
    index_base: usize,
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
pub(super) fn about_origin(local: Affine, origin: [f32; 2]) -> AffineTransform {
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

pub(super) fn image_quad(
    source: &Arc<str>,
    fit: ImageFit,
    sampling: nana_ui_core::ImageSampling,
    radii: [f32; 4],
) -> ScenePrimitiveKind {
    let mut image = BackgroundImage::url_with_fit(
        source.as_ref(),
        match fit {
            ImageFit::Fill => nana_ui_core::BackgroundImageFit::Stretch,
            ImageFit::Contain => nana_ui_core::BackgroundImageFit::Contain,
            ImageFit::Cover => nana_ui_core::BackgroundImageFit::Cover,
            ImageFit::None => nana_ui_core::BackgroundImageFit::Auto,
            ImageFit::ScaleDown => nana_ui_core::BackgroundImageFit::ScaleDown,
        },
    )
    .with_sampling(sampling);
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
        surface: Box::new(QuadSurfacePaint {
            content_image: Some(image),
            ..QuadSurfacePaint::default()
        }),
    }
}

pub(super) fn custom_text(
    node: &ExtractedNode,
    text: &TextStyle,
    color: [f32; 4],
    paint_color: Option<nana_ui_core::PaintColor>,
) -> ScenePrimitiveKind {
    ScenePrimitiveKind::Text {
        content: text.content.to_string().into(),
        color: super::slot_color(paint_color, Some(color)),
        size: text.size,
        weight: text.weight,
        family: text
            .family
            .as_deref()
            .or(node.style.font_family.as_deref())
            .map(str::to_owned),
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
        rich: None,
        presentation: None,
    }
}

/// The box around `rects`, which is not empty.
pub(super) fn union_rect(rects: &[SceneRect]) -> SceneRect {
    let [mut left, mut top] = [f32::INFINITY; 2];
    let [mut right, mut bottom] = [f32::NEG_INFINITY; 2];
    for rect in rects {
        left = left.min(rect.x);
        top = top.min(rect.y);
        right = right.max(rect.x + rect.width);
        bottom = bottom.max(rect.y + rect.height);
    }
    SceneRect {
        x: left,
        y: top,
        width: right - left,
        height: bottom - top,
    }
}

pub(super) fn offset_rect(rect: SceneRect, origin: [f32; 2]) -> SceneRect {
    SceneRect {
        x: rect.x + origin[0],
        y: rect.y + origin[1],
        ..rect
    }
}

pub(super) fn clip_region(
    clip: &LocalClip,
    origin: [f32; 2],
    transform: AffineTransform,
) -> ClipRegion {
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
pub(super) struct ActiveClip {
    /// What the clip was built from: every clip path pushed so far, with its
    /// transform. Part of the key of everything built under it.
    key: Arc<[u32]>,
    built: Arc<BuiltClip>,
}

/// A clip stack entry's geometry, kept across recordings like meshes are.
#[derive(Debug)]
pub(super) struct BuiltClip {
    shapes: Shapes,
    /// The clip, triangulated, for cutting meshes that cannot be re-derived
    /// from a polygon (shadow bands carry a coverage ramp).
    triangles: Vec<[[f32; 2]; 3]>,
    /// The same clips as GPU clips, when every one of them is exact.
    /// `None`: content the CPU cannot cut goes through a masked layer.
    local: Option<Arc<[LocalClip]>>,
    /// What a layer cut to this clip erases, once something needed it.
    erase: OnceLock<Option<Arc<PathMesh>>>,
}

pub(super) fn run_quads(built: Vec<BuiltOp>) -> Vec<BuiltOp> {
    fn solid(op: &BuiltOp) -> Option<(SceneRect, [f32; 4], [f32; 4], Affine)> {
        match op {
            BuiltOp::Quad {
                rect,
                radii,
                fill: Some(fill),
                fill_color_space: None,
                border: None,
                shadow: None,
                transform,
                ..
            } => Some((*rect, *fill, *radii, *transform)),
            _ => None,
        }
    }
    let mut out: Vec<BuiltOp> = Vec::with_capacity(built.len());
    for op in built {
        let Some((rect, fill, radii, transform)) = solid(&op) else {
            out.push(op);
            continue;
        };
        match out.last_mut() {
            Some(BuiltOp::QuadRun {
                items,
                radii: run_radii,
                transform: run_transform,
            }) if *run_radii == radii && *run_transform == transform => {
                items.push((rect, fill));
            }
            Some(last)
                if solid(last).is_some_and(|(_, _, last_radii, last_transform)| {
                    last_radii == radii && last_transform == transform
                }) =>
            {
                let (first, first_fill, ..) = solid(last).expect("checked above");
                *last = BuiltOp::QuadRun {
                    items: vec![(first, first_fill), (rect, fill)],
                    radii,
                    transform,
                };
            }
            _ => out.push(op),
        }
    }
    out
}

/// The rectangle and corner radii of a path that is one (rounded)
/// rectangle with room to draw.
pub(super) fn quad_shape(path: &PaintPath) -> Option<(SceneRect, [f32; 4])> {
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
pub(super) fn stroke_quad_radii(radii: [f32; 4], stroke: &StrokeStyle) -> Option<[f32; 4]> {
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

pub(super) fn affine_apply(t: Affine, p: [f32; 2]) -> [f32; 2] {
    let [a, b, c, d, e, f] = t;
    [a * p[0] + c * p[1] + e, b * p[0] + d * p[1] + f]
}

pub(super) fn affine_inverse(t: Affine) -> Option<Affine> {
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
pub(super) fn affine_scale(t: Affine) -> f32 {
    let [a, b, c, d, _, _] = t;
    let fro2 = a * a + b * b + c * c + d * d;
    let det = a * d - b * c;
    let disc = (fro2 * fro2 - 4.0 * det * det).max(0.0);
    ((fro2 + disc.sqrt()) * 0.5).max(0.0).sqrt()
}

/// The path as a GPU clip, when the GPU clip can express it exactly: a
/// (rounded) rectangle or a polygon of up to eight vertices.
pub(super) fn local_clip(path: &PaintPath, shapes: &Shapes, t: Affine) -> Option<LocalClip> {
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

pub(super) fn scene_rect(rect: LayoutBox) -> SceneRect {
    SceneRect {
        x: rect.x,
        y: rect.y,
        width: rect.width.max(0.0),
        height: rect.height.max(0.0),
    }
}

pub(super) fn shapes_bounds(shapes: &Shapes) -> Option<SceneRect> {
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
