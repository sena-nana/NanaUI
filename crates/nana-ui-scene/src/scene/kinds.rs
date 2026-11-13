//! The primitive record and its kinds.

use super::*;

#[derive(Debug, Clone, PartialEq, Default)]
pub struct QuadSurfacePaint {
    pub presentation_masks: Vec<nana_ui_core::PresentationMaskSample>,
    pub background_material: Option<nana_ui_core::MaterialRef>,
    pub border_material: Option<nana_ui_core::MaterialRef>,
    pub background_image: Option<BackgroundImage>,
    /// Extra CSS `background-image` layers after the first (below it).
    pub background_layers: Vec<BackgroundImage>,
    /// `<img src>` replaced content, painted above background layers.
    pub content_image: Option<BackgroundImage>,
    pub mask: Option<nana_ui_core::MaskImage>,
    /// Resolved polygon vertices in border-box coordinates (px).
    pub polygon_clip: Option<Vec<[f32; 2]>>,
    pub filter: Option<ColorFilter>,
    pub backdrop_filter: Option<nana_ui_core::BackdropFilter>,
    /// Extra `box-shadow` layers after the primary (GPU cap 4 including primary).
    pub extra_shadows: Vec<ComponentElevation>,
    pub outline_width: f32,
    pub outline_color: Option<nana_ui_core::PaintColor>,
    pub mix_blend: MixBlendMode,
    /// Per-side stroke (T,R,B,L). All-zero keeps [`ScenePrimitiveKind::Quad::border_width`].
    pub border_widths: [f32; 4],
    /// Per-side colors (T,R,B,L). `None` falls back to the quad `border_color`.
    pub border_colors: [Option<nana_ui_core::PaintColor>; 4],
    /// Per-side shader style (T,R,B,L): 0 solid, 1 dashed, 2 dotted.
    pub border_styles: [u8; 4],
    /// Minimal `border-image` 9-slice (`url()` / linear-gradient + slice).
    pub border_image: Option<BorderImageSpec>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ScenePrimitiveKind {
    Quad {
        background: Option<nana_ui_core::PaintColor>,
        border_color: Option<nana_ui_core::PaintColor>,
        border_width: f32,
        corner_radius: [f32; 4],
        shadow: Option<ComponentElevation>,
        /// Images, masks, materials and filters: rare, and over a kilobyte,
        /// so boxed to keep every primitive (and every frame building one)
        /// small.
        surface: Box<QuadSurfacePaint>,
    },
    QuadBatch {
        bounds: Vec<SceneRect>,
        background: Option<nana_ui_core::PaintColor>,
        border_color: Option<nana_ui_core::PaintColor>,
        border_width: f32,
        corner_radius: [f32; 4],
        shadow: Option<ComponentElevation>,
        surface: Box<QuadSurfacePaint>,
    },
    /// One batch of solid-color quads with per-item colors (editor color
    /// swatch decorators). Mirrors [`ScenePrimitiveKind::QuadBatch`]: one
    /// scene slot regardless of count and color variety, so a large set
    /// never saturates `u8` slot indices. No shadow and no surface paint —
    /// decorative overlays only.
    QuadColorBatch {
        bounds: Vec<SceneRect>,
        colors: Vec<[f32; 4]>,
        border_color: Option<[f32; 4]>,
        border_width: f32,
        corner_radius: [f32; 4],
    },
    Text {
        content: nana_ui_runtime::TextValue,
        color: Option<nana_ui_core::PaintColor>,
        size: f32,
        weight: Option<u16>,
        family: Option<String>,
        line_height: Option<LineHeightSpec>,
        letter_spacing: f32,
        wrap: bool,
        ellipsis: bool,
        max_lines: Option<u16>,
        shaping: TextShaping,
        horizontal_alignment: TextHorizontalAlignment,
        vertical_alignment: TextVerticalAlignment,
        /// Theme-resolved committed-text spans. Empty means solid `color`.
        spans: Vec<SceneTextSpan>,
        text_shadow: Option<nana_ui_core::TextShadowSpec>,
        underline: bool,
        line_through: bool,
        font_features: Vec<nana_ui_core::FontFeatureSetting>,
        italic: bool,
        wrap_break: nana_ui_core::TextWrapBreak,
        /// OpenType / wrap subset from computed style. Defaults are CSS initial.
        opentype: SceneTextOpenType,
        /// The `nana-text` layout Runtime measured this text with, when the
        /// host resolved plain text through an engine (Issue #95). Present, it
        /// is the placement authority for these glyphs; the fields above stay
        /// for renderers that still lay text out themselves. Never a shaping
        /// backend's buffer.
        layout: Option<nana_ui_runtime::RetainedTextLayout>,
        /// Decoration lines, outline and shadow layers, by byte range. When
        /// present it is what the painter draws them from; `text_shadow`,
        /// `underline` and `line_through` above stay for other readers.
        rich: Option<Arc<SceneRichPaint>>,
        /// Per-glyph presentation: effects and a reveal the text shader
        /// samples on the motion clock. Not part of what the glyphs were
        /// built from, so changing it rebuilds no instance.
        presentation: Option<Arc<SceneGlyphPresentation>>,
    },
    Icon {
        icon: Icon,
        color: Option<nana_ui_core::PaintColor>,
    },
    /// Many instances of one icon in a single primitive. Mirrors
    /// [`ScenePrimitiveKind::QuadBatch`]: one scene slot regardless of count,
    /// so a large set (editor whitespace tab arrows) never saturates `u8`
    /// slot indices.
    IconBatch {
        bounds: Vec<SceneRect>,
        icon: Icon,
        color: Option<[f32; 4]>,
    },
    Spinner {
        phase: u8,
        color: Option<nana_ui_core::PaintColor>,
    },
    Stroke {
        points: Vec<[f32; 2]>,
        width: f32,
        color: [f32; 4],
        /// Per-point stroke widths. Empty means every vertex uses [`Self::Stroke::width`].
        widths: Vec<f32>,
        cap: StrokeCap,
        /// Dash and per-point colors. `None` is the Graph / TimeSeries path:
        /// no extra heap and the painter keeps the solid uniform emit.
        pattern: Option<Box<StrokePattern>>,
    },
    Custom {
        node: CustomRenderNode,
        /// `mask-image` / `-webkit-mask-image` alpha for HostTexture sampling.
        /// Same value as [`QuadSurfacePaint::mask`] (gradient or `url()`).
        mask: Option<nana_ui_core::MaskImage>,
        /// The rounding of the primitive's own box, in [`Self::Quad`]'s
        /// corner order. A HostTexture draws its content in this shape.
        corner_radius: [f32; 4],
    },
    /// Triangles a node's `Painter` produced (Issue #217): path fills,
    /// strokes and shadows, already clipped and anti-aliased on the CPU.
    /// Vertices are node-local; `origin` places them in the node's layout
    /// space, so a node that only moves keeps its mesh.
    Path {
        mesh: Arc<PathMesh>,
        origin: [f32; 2],
        /// The node's `mask-image`, multiplied into every fragment of the
        /// mesh. See [`NodeMask`].
        node_mask: Option<Arc<NodeMask>>,
    },
    /// A painter layer opens (Issue #217): what the node paints from here to
    /// the matching [`Self::LayerEnd`] draws into its own layer, which then
    /// composites with `opacity` and `blend`, cut to `clip` (layout space,
    /// under the primitive's transform) when there is one.
    LayerBegin {
        opacity: f32,
        blend: MixBlendMode,
        clip: Option<SceneRect>,
    },
    /// The layer closes, after `mask` has been applied to it.
    LayerEnd { mask: Option<LayerMask> },
    /// A chart's marks (`nana-ui-charts`), drawn by the chart shaders from
    /// arrays the painter keeps on the GPU while `marks.revision` holds.
    /// Marks are node-local; `origin` places them in layout space. Motion
    /// and hover emphasis are sampled on the motion clock, so neither
    /// rebuilds anything per frame.
    Chart {
        marks: nana_ui_charts::ChartMarks,
        origin: [f32; 2],
        hover: SceneChartHover,
    },
}

/// What a chart's shaders emphasise, and since when.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SceneChartHover {
    /// `(series, index)` emphasised now and before.
    pub current: [u32; 2],
    pub previous: [u32; 2],
    /// Focused series now and before.
    pub focus: [u32; 2],
    pub since: std::time::Duration,
    /// Seconds an emphasis change takes.
    pub duration: f32,
    /// px a hovered slice grows.
    pub growth: f32,
}

/// A mesh applied to a painter layer before it composites.
#[derive(Debug, Clone, PartialEq)]
pub struct LayerMask {
    pub mesh: Arc<PathMesh>,
    /// Places the node-local mesh in layout space, as for
    /// [`ScenePrimitiveKind::Path`].
    pub origin: [f32; 2],
    pub mode: LayerMaskMode,
    /// The node's `mask-image`, for [`LayerMaskMode::Keep`]; `None` for the
    /// other modes.
    pub node_mask: Option<Arc<NodeMask>>,
}

/// What a [`LayerMask`] does to the layer under it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LayerMaskMode {
    /// Remove the layer where the mesh covers it.
    Erase,
    /// Recolour the layer with the mesh's paint, keeping the layer's alpha.
    Tint,
    /// Scale the layer by [`LayerMask::node_mask`] where the mesh covers it.
    /// Glyphs, icons and images a painter draws on a masked node take the
    /// node's mask this way: their pipelines do not evaluate one.
    Keep,
}

/// A node's `mask-image` as its `Painter`'s output takes it (Issue #217).
///
/// The mask lies over the node's own layout box exactly as it does over the
/// node's own quad: node-local `p` samples it at `p / size`, so a painted
/// shape and the node's background fade alike. Past the box — a shadow or a
/// glow reaching out — it holds the value at the box's edge (a gradient's
/// end stops), where CSS `mask-clip: border-box` would cut everything off.
///
/// Only gradients: a `url()` mask is not applied to painter output (the node
/// counts it in `runtime.paint.mask_unsupported`).
#[derive(Debug, Clone, PartialEq)]
pub struct NodeMask {
    pub gradient: nana_ui_core::CssGradient,
    /// The node's layout size, logical px, both axes positive.
    pub size: [f32; 2],
}

impl ScenePrimitiveKind {
    /// Only Quad pipelines bind `motion.wgsl` `evaluate()`. Text / Icon /
    /// Mesh / HostTexture still consume CPU compositor presentation.
    pub fn evaluates_compositor_motion_on_gpu(&self) -> bool {
        matches!(
            self,
            Self::Quad { .. } | Self::QuadBatch { .. } | Self::QuadColorBatch { .. }
        )
    }

    /// Returns `true` when this primitive carries wide-gamut or HDR authoring
    /// colors (or amplifying filters) that require a floating-point linear
    /// working buffer rather than `[0, 1]`-clamping `Rgba8UnormSrgb`.
    pub fn has_extended_color(&self) -> bool {
        match self {
            Self::Quad {
                background,
                border_color,
                shadow,
                surface,
                ..
            }
            | Self::QuadBatch {
                background,
                border_color,
                shadow,
                surface,
                ..
            } => {
                background.is_some_and(nana_ui_core::PaintColor::is_extended_srgb)
                    || border_color.is_some_and(nana_ui_core::PaintColor::is_extended_srgb)
                    || shadow.as_ref().is_some_and(|s| s.color.is_extended_srgb())
                    || surface_has_extended_color(surface)
            }
            Self::Text {
                color,
                spans,
                text_shadow,
                rich,
                ..
            } => {
                color.is_some_and(nana_ui_core::PaintColor::is_extended_srgb)
                    || spans.iter().any(|s| s.color.is_extended_srgb())
                    || text_shadow.as_ref().is_some_and(|s| {
                        s.paint_color
                            .is_some_and(nana_ui_core::PaintColor::is_extended_srgb)
                    })
                    || rich
                        .as_ref()
                        .is_some_and(|r| rich_paint_has_extended_color(r))
            }
            Self::Icon { color, .. } | Self::Spinner { color, .. } => {
                color.is_some_and(nana_ui_core::PaintColor::is_extended_srgb)
            }
            Self::Path { mesh, .. } => path_mesh_has_extended_color(mesh),
            Self::LayerEnd { mask } => mask
                .as_ref()
                .is_some_and(|m| path_mesh_has_extended_color(&m.mesh)),
            Self::QuadColorBatch { .. }
            | Self::IconBatch { .. }
            | Self::Stroke { .. }
            | Self::Custom { .. }
            | Self::LayerBegin { .. }
            | Self::Chart { .. } => false,
        }
    }
}

pub(super) fn color_filter_has_extended_color(filter: &ColorFilter) -> bool {
    const EPS: f32 = 1.0e-3;
    filter.brightness > 1.0 + EPS
        || filter.saturate > 1.0 + EPS
        || filter.drop_shadow.as_ref().is_some_and(|s| {
            s.paint_color
                .is_some_and(nana_ui_core::PaintColor::is_extended_srgb)
        })
}

fn background_image_has_extended_color(image: &BackgroundImage) -> bool {
    match image {
        BackgroundImage::Gradient(nana_ui_core::CssGradient::Linear(g)) => {
            g.stops.iter().any(|s| {
                s.paint_color
                    .is_some_and(nana_ui_core::PaintColor::is_extended_srgb)
            })
        }
        BackgroundImage::Gradient(nana_ui_core::CssGradient::Radial(g)) => {
            g.stops.iter().any(|s| {
                s.paint_color
                    .is_some_and(nana_ui_core::PaintColor::is_extended_srgb)
            })
        }
        BackgroundImage::Url { .. } => false,
    }
}

fn surface_has_extended_color(surface: &QuadSurfacePaint) -> bool {
    surface
        .background_image
        .as_ref()
        .is_some_and(background_image_has_extended_color)
        || surface
            .background_layers
            .iter()
            .any(background_image_has_extended_color)
        || surface
            .content_image
            .as_ref()
            .is_some_and(background_image_has_extended_color)
        || surface
            .filter
            .as_ref()
            .is_some_and(color_filter_has_extended_color)
        || surface
            .backdrop_filter
            .as_ref()
            .is_some_and(|bf| bf.saturate > 1.0 + 1.0e-3)
        || surface
            .extra_shadows
            .iter()
            .any(|s| s.color.is_extended_srgb())
        || surface
            .outline_color
            .is_some_and(nana_ui_core::PaintColor::is_extended_srgb)
        || surface
            .border_colors
            .iter()
            .flatten()
            .any(|c| c.is_extended_srgb())
        || surface
            .border_image
            .as_ref()
            .is_some_and(|bi| background_image_has_extended_color(&bi.source))
}

fn text_effects_has_extended_color(fx: &SceneTextEffects) -> bool {
    fx.decoration_color
        .is_some_and(nana_ui_core::PaintColor::is_extended_srgb)
        || fx
            .emphasis_color
            .is_some_and(nana_ui_core::PaintColor::is_extended_srgb)
        || fx.stroke.is_some_and(|s| s.color.is_extended_srgb())
        || fx.shadows.iter().any(|s| s.color.is_extended_srgb())
}

fn rich_paint_has_extended_color(rich: &SceneRichPaint) -> bool {
    text_effects_has_extended_color(&rich.base)
        || rich
            .runs
            .iter()
            .any(|run| text_effects_has_extended_color(&run.effects))
}

fn path_mesh_has_extended_color(mesh: &PathMesh) -> bool {
    const EPS: f32 = 1.0e-3;
    mesh.paint_color
        .is_some_and(nana_ui_core::PaintColor::is_extended_srgb)
        || mesh
            .gradient
            .as_ref()
            .and_then(|g| g.linear_stops.as_ref())
            .is_some_and(|stops| {
                stops.iter().any(|(_, [r, g, b, _])| {
                    [*r, *g, *b]
                        .into_iter()
                        .any(|c| c.is_finite() && (c < -EPS || c > 1.0 + EPS))
                })
            })
}

/// Optional dash and per-point colors for [`ScenePrimitiveKind::Stroke`].
///
/// Empty `dash` is solid. Empty `colors` uses the stroke's uniform `color`.
/// The painter only walks these slices when they are non-empty, so unused
/// decorations do not add GPU instance fields or shader work. Graph /
/// TimeSeries keep [`ScenePrimitiveKind::Stroke::pattern`] as `None`.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct StrokePattern {
    /// SVG-style on/off lengths. Negative or non-finite values disable dash
    /// (treated as solid). A single-cycle odd list is repeated to even length.
    pub dash: Vec<f32>,
    pub dash_offset: f32,
    /// SVG `pathLength` for dasharray/dashoffset. Zero, negative, or
    /// non-finite is unset (use geometric length). Dashes are in these units:
    /// geometric `s` maps to `s * (path_length / geometric_length)` before
    /// phase. Ignored when `dash` is empty (solid). Scene Stroke callers set
    /// this field; CSS does not extract it (generic SVG is resvg).
    pub path_length: f32,
    /// Per-point colors. Used only when `len` matches the stroke point count;
    /// each segment takes the color of its start vertex.
    pub colors: Vec<[f32; 4]>,
}

/// End-cap of an articulated stroke segment.
///
/// Round is the Ciallo vanilla disc. Butt is a flat cut at the endpoint.
/// Square extends half-width past the endpoint, then cuts flat. The painter
/// expands Square on the CPU and reuses the Butt GPU path.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum StrokeCap {
    #[default]
    Round,
    Butt,
    Square,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ScenePrimitive {
    pub id: PrimitiveId,
    pub node: StableNodeId,
    pub bounds: SceneRect,
    pub transform: AffineTransform,
    /// Ancestor clip chain, shared: every primitive of a node, and every node
    /// under the same clipping ancestors, points at one allocation.
    pub clips: Arc<[ClipRegion]>,
    /// Paint opacity excluding ancestor opacity groups.
    pub opacity: f32,
    pub z_index: i32,
    pub document_order: usize,
    pub kind: ScenePrimitiveKind,
}

// Every primitive the scene holds or a builder makes is this size, and an
// unoptimized build gives each one its own stack slot: inlining a large rare
// field (the quad surface was 1 KiB) grew the builders' frames past 150 KiB
// each. Box such fields instead.
const _: () = assert!(std::mem::size_of::<ScenePrimitive>() <= 512);
