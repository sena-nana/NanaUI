mod attributes;
mod composition;
mod compositor;
use attributes::DrawAttributes;
pub use attributes::SceneDraw;
use compositor::CompositorRegistry;
pub use compositor::{
    CompositorLayer, CompositorLayerId, CompositorLayerKind, CompositorMotionBinding,
    CompositorPaintEncode, LAYER_DEMOTE_HOLD, LAYER_PROMOTE_HOLD,
};
mod visibility;
pub use composition::FramePlan;
use visibility::VisibilityIndex;
mod custom_paint;
pub use custom_paint::{PathMesh, PathVertex};
mod order;
mod primitives;
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::sync::{Arc, OnceLock};

use nana_ui_core::{
    BackgroundImage, BorderImageSpec, ClipPath, ColorFilter, ControlSize, DirSpec, DrawerSide,
    FontFeatureSetting, FontKerningSpec, FontVariationSetting, Icon, LineBreakSpec, LineHeightSpec,
    MixBlendMode, SwitchControlPosition, WordBreakSpec, WritingModeSpec,
    icon_y_on_text_glyph_center,
};
use nana_ui_runtime::{
    ComponentElevation, ComponentGeometry, ComponentTextRegion, CustomRenderNode, ExtractedNode,
    LayoutBox, NodeKind, NodeMap, NodeSet, StableNodeId, StandardVisual, TextFoldGutter,
    TextHorizontalAlignment, TextInputScroll, TextShaping, TextVerticalAlignment,
    TextWhitespaceKind,
};

use crate::{
    AccessMode, CompiledRenderGraph, GraphError, PassId, RenderGraph, RenderOperation, RenderPass,
    RenderResource, ResourceAccess, ResourceId,
};

const fn corner_radii(r: f32) -> [f32; 4] {
    [r; 4]
}

fn focus_ring_corner_radius(
    style: &nana_ui_core::LayoutStyle,
    bounds: SceneRect,
    outset: f32,
) -> [f32; 4] {
    let radii = style.resolved_border_radii(bounds.width, bounds.height);
    let max_r = radii.into_iter().fold(0.0f32, f32::max);
    corner_radii(max_r + outset)
}

#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct SceneRect {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AffineTransform(pub [f32; 6], pub [f32; 2]);

impl AffineTransform {
    pub const IDENTITY: Self = Self([1.0, 0.0, 0.0, 1.0, 0.0, 0.0], [0.0, 0.0]);

    pub const fn from_matrix(matrix: [f32; 6]) -> Self {
        Self(matrix, [0.0, 0.0])
    }

    pub fn is_projective(self) -> bool {
        self.1[0].abs() > 1e-8 || self.1[1].abs() > 1e-8
    }

    pub fn then(self, rhs: Self) -> Self {
        let [a, b, c, d, e, f] = self.0;
        let [g, h] = self.1;
        let [ra, rb, rc, rd, re, rf] = rhs.0;
        let [rg, rh] = rhs.1;
        let na = a * ra + c * rb + e * rg;
        let nb = b * ra + d * rb + f * rg;
        let nc = a * rc + c * rd + e * rh;
        let nd = b * rc + d * rd + f * rh;
        let ne = a * re + c * rf + e;
        let nf = b * re + d * rf + f;
        let ng = g * ra + h * rb + rg;
        let nh = g * rc + h * rd + rh;
        let ni = g * re + h * rf + 1.0;
        // Keep the CPU contract wide enough to match WGSL f32/FMA rounding.
        if !ni.is_finite() || ni.abs() < 1e-6 {
            return Self::IDENTITY;
        }
        let inv = 1.0 / ni;
        Self(
            [na * inv, nb * inv, nc * inv, nd * inv, ne * inv, nf * inv],
            [ng * inv, nh * inv],
        )
    }
}

impl From<[f32; 6]> for AffineTransform {
    fn from(matrix: [f32; 6]) -> Self {
        Self::from_matrix(matrix)
    }
}

impl Default for AffineTransform {
    fn default() -> Self {
        Self::IDENTITY
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct ClipRegion {
    pub bounds: SceneRect,
    pub transform: AffineTransform,
    /// Rounded inset clip radius (px in border-box space). Zero = axis-aligned rect.
    pub corner_radius: f32,
    /// `clip-path: polygon(...)` vertices in [`Self::bounds`] local space (px).
    pub polygon_clip: Option<Vec<[f32; 2]>>,
}

impl ClipRegion {
    pub fn axis_aligned(bounds: SceneRect, transform: AffineTransform) -> Self {
        Self {
            bounds,
            transform,
            corner_radius: 0.0,
            polygon_clip: None,
        }
    }

    /// Ellipse filling `bounds` (`clip-path: circle()` / `ellipse()`).
    pub fn ellipse(bounds: SceneRect, transform: AffineTransform) -> Self {
        Self {
            bounds,
            transform,
            corner_radius: 0.0,
            polygon_clip: Some(vec![[f32::NEG_INFINITY, f32::NEG_INFINITY]]),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PrimitiveId {
    pub node: StableNodeId,
    /// Fixed component slots occupy 0..=255. Unbounded component collections
    /// use a separate namespace and a checked collection index.
    pub slot: u64,
}

fn collection_slot(namespace: u32, index: usize) -> u64 {
    debug_assert!(namespace != 0);
    (u64::from(namespace) << 32)
        | u64::from(u32::try_from(index).expect("primitive collection exceeds u32::MAX items"))
}

/// `::selection` fills share the custom-render slot's paint layer so they stay
/// behind the glyphs, but live in their own namespace so the two cannot
/// overwrite each other's key.
const DOCUMENT_TEXT_SELECTION: u32 = 6;
const TEXT_LINE_LABELS: u32 = 1;
const TEXT_DIAGNOSTIC_MARKERS: u32 = 2;
const TEXT_DIAGNOSTIC_LABELS: u32 = 3;
const TEXT_ATOM_ICONS: u32 = 4;
const TEXT_ATOM_LABELS: u32 = 5;
/// Inline objects of a rich text node, one slot per object in text order.
const TEXT_INLINE_OBJECTS: u32 = 7;
/// A rich text editor's caret, over its glyphs.
const TEXT_EDITOR_CARET: u32 = 8;
/// A rich text editor's selection rectangles share the document selection's
/// paint layer (under the glyphs) from this index on.
const RICH_EDITOR_SELECTION_BASE: usize = 1 << 16;
/// Rich text tag backgrounds share that paint layer too (under the glyphs,
/// over a selection), from this index on.
const RICH_TAG_BASE: usize = 1 << 17;

/// A tag's fill by its kind: the application's marker kinds, cycled.
fn tag_color(kind: u16) -> [f32; 4] {
    const PALETTE: [[f32; 4]; 6] = [
        [0.93, 0.55, 0.20, 0.92],
        [0.36, 0.52, 0.92, 0.92],
        [0.55, 0.40, 0.85, 0.92],
        [0.22, 0.66, 0.55, 0.92],
        [0.88, 0.38, 0.55, 0.92],
        [0.45, 0.50, 0.58, 0.92],
    ];
    PALETTE[usize::from(kind) % PALETTE.len()]
}

/// The surface of an open triggered menu (Popover, ActionMenu, HoverCard).
/// It is the trigger's primitive, but it wraps content Runtime lays out
/// viewport-fixed above the page, so it paints the way that content does: in
/// the root stacking context, untransformed, and cut by none of the trigger's
/// clips. It keeps no projection of the trigger's to rebase on either, so a
/// scroll under the trigger leaves it where its content is.
const TRIGGERED_OVERLAY_SURFACE: u32 = 0xFFFF_FFE0;
pub(super) const TRIGGERED_OVERLAY_SURFACE_SLOT: u64 = (TRIGGERED_OVERLAY_SURFACE as u64) << 32;

fn is_triggered_overlay_surface(id: PrimitiveId) -> bool {
    id.slot == TRIGGERED_OVERLAY_SURFACE_SLOT
}

/// A triggered surface's paint-order key: its own `(z_index, document_order)`
/// at the root, where `z_index` is the menu content's level, so it sits under
/// the content (later in document order) and over its trigger and the rest
/// of the page, whatever groups the trigger is inside.
fn triggered_overlay_surface_key(primitive: &ScenePrimitive) -> Option<SceneOrderKey> {
    is_triggered_overlay_surface(primitive.id).then(|| {
        SceneOrderKey::at(
            Arc::from([(primitive.z_index, primitive.document_order)]),
            primitive,
        )
    })
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct QuadSurfacePaint {
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
        surface: QuadSurfacePaint,
    },
    QuadBatch {
        bounds: Vec<SceneRect>,
        background: Option<nana_ui_core::PaintColor>,
        border_color: Option<nana_ui_core::PaintColor>,
        border_width: f32,
        corner_radius: [f32; 4],
        shadow: Option<ComponentElevation>,
        surface: QuadSurfacePaint,
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
}

/// What a [`LayerMask`] does to the layer under it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LayerMaskMode {
    /// Remove the layer where the mesh covers it.
    Erase,
    /// Recolour the layer with the mesh's paint, keeping the layer's alpha.
    Tint,
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
    /// this field; Vue/CSS does not extract it (generic SVG is resvg).
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

/// One paint-ready span inside a [`ScenePrimitiveKind::Text`] primitive.
#[derive(Debug, Clone, PartialEq)]
pub struct SceneTextSpan {
    pub start: usize,
    pub end: usize,
    /// In the space it was authored in, converted once at GPU upload.
    pub color: nana_ui_core::PaintColor,
}

/// What a stretch of a [`ScenePrimitiveKind::Text`] draws besides its fill:
/// decoration lines, an outline, shadow layers. The fill colour itself rides
/// in [`ScenePrimitiveKind::Text::spans`], like any other coloured run.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct SceneTextEffects {
    pub decoration: nana_ui_core::TextDecorationLine,
    /// `None` draws the lines in the glyphs' own fill.
    pub decoration_color: Option<nana_ui_core::PaintColor>,
    pub stroke: Option<nana_ui_core::RichTextStroke>,
    /// CSS order: the first is drawn on top. At most
    /// [`nana_ui_core::MAX_TEXT_SHADOWS`].
    pub shadows: Arc<[nana_ui_core::RichTextShadow]>,
}

impl SceneTextEffects {
    pub fn is_empty(&self) -> bool {
        !self.decoration.is_active() && self.stroke.is_none() && self.shadows.is_empty()
    }

    /// Logical px these effects can reach past the glyphs' own ink.
    pub fn reach(&self) -> f32 {
        let shadows = self
            .shadows
            .iter()
            .map(|shadow| {
                shadow.offset[0].abs().max(shadow.offset[1].abs())
                    + shadow.blur_px.max(0.0)
                    + shadow.spread_px.max(0.0)
            })
            .fold(0.0f32, f32::max);
        let stroke = self
            .stroke
            .map_or(0.0, |stroke| stroke.width_px.max(0.0) * 0.5);
        shadows.max(stroke) + stroke
    }

    /// `over` laid on these effects: each one `over` sets wins.
    pub fn overlay(&self, over: &nana_ui_core::RichPaintStyle) -> Self {
        Self {
            decoration: over.decoration.unwrap_or(self.decoration),
            decoration_color: over.decoration_color.or(self.decoration_color),
            stroke: over.stroke.or(self.stroke),
            shadows: over
                .shadows
                .clone()
                .unwrap_or_else(|| Arc::clone(&self.shadows)),
        }
    }
}

/// What a Text primitive presents per glyph, by grapheme ordinal: the
/// effect table, which effect each grapheme plays, and the reveal.
#[derive(Debug, Clone, PartialEq)]
pub struct SceneGlyphPresentation {
    pub effects: Arc<[nana_ui_core::GlyphEffect]>,
    /// Effect index per grapheme of the content; `u16::MAX` for none. Empty
    /// when no grapheme plays an effect.
    pub cluster_effects: Arc<[u16]>,
    pub reveal: Option<nana_ui_core::RevealSchedule>,
}

impl SceneGlyphPresentation {
    /// The presentation of `content` under `presentation`, its graphemes'
    /// effect indices read from `rich`'s spans.
    pub fn new(
        content: &str,
        rich: Option<&nana_ui_core::RichText>,
        presentation: &nana_ui_core::GlyphPresentation,
    ) -> Option<Arc<Self>> {
        use unicode_segmentation::UnicodeSegmentation;
        let mut any = false;
        let cluster_effects: Vec<u16> = match rich.filter(|rich| rich.text() == content) {
            Some(rich) if !presentation.effects.is_empty() => content
                .grapheme_indices(true)
                .map(
                    |(offset, _)| match rich.style_at(offset).and_then(|style| style.effect) {
                        Some(effect) if usize::from(effect) < presentation.effects.len() => {
                            any = true;
                            effect
                        }
                        _ => u16::MAX,
                    },
                )
                .collect(),
            _ => Vec::new(),
        };
        if !any && presentation.reveal.is_none() {
            return None;
        }
        Some(Arc::new(Self {
            effects: Arc::clone(&presentation.effects),
            cluster_effects: if any {
                cluster_effects.into()
            } else {
                Arc::from([])
            },
            reveal: presentation.reveal.clone(),
        }))
    }

    /// Until when it changes what is drawn; `None` while an effect loops.
    pub fn live_until(&self) -> Option<std::time::Duration> {
        if !self.cluster_effects.is_empty() {
            return None;
        }
        self.reveal.as_ref().map(nana_ui_core::RevealSchedule::end)
    }

    /// The effect grapheme `ordinal` plays.
    pub fn effect_of(&self, ordinal: usize) -> Option<&nana_ui_core::GlyphEffect> {
        self.cluster_effects
            .get(ordinal)
            .filter(|index| **index != u16::MAX)
            .and_then(|index| self.effects.get(usize::from(*index)))
    }
}

/// One byte range of a Text primitive with effects of its own.
#[derive(Debug, Clone, PartialEq)]
pub struct SceneRichRun {
    pub range: std::ops::Range<usize>,
    pub effects: SceneTextEffects,
}

/// Everything a Text primitive draws besides its fill, resolved: the node's
/// own effects (CSS `text-decoration`, `text-shadow`, `-webkit-text-stroke`)
/// and the ranges a rich text overrides them over.
///
/// The painter turns this into instances of the same paragraph — shadows,
/// then strokes, then fills, then lines — so a paint change is a rebuild of
/// those instances and never a relayout.
#[derive(Debug, Clone, PartialEq)]
pub struct SceneRichPaint {
    pub base: SceneTextEffects,
    /// Sorted, non-overlapping.
    pub runs: Vec<SceneRichRun>,
    /// A hash of all of the above. Equal effects have equal revisions, so the
    /// painter can tell a repaint that changed nothing from one that did
    /// without comparing them. Never zero.
    pub revision: u64,
    /// Logical px the effects can reach past the glyphs' own ink.
    pub reach: f32,
}

impl SceneRichPaint {
    /// `None` when nothing has an effect.
    pub fn new(base: SceneTextEffects, runs: Vec<SceneRichRun>) -> Option<Arc<Self>> {
        let runs: Vec<SceneRichRun> = runs
            .into_iter()
            .filter(|run| run.range.start < run.range.end && run.effects != base)
            .collect();
        if base.is_empty() && runs.iter().all(|run| run.effects.is_empty()) {
            return None;
        }
        let reach = runs
            .iter()
            .map(|run| run.effects.reach())
            .fold(base.reach(), f32::max);
        let mut hasher = std::hash::DefaultHasher::new();
        hash_effects(&base, &mut hasher);
        for run in &runs {
            std::hash::Hash::hash(&run.range, &mut hasher);
            hash_effects(&run.effects, &mut hasher);
        }
        let revision = std::hash::Hasher::finish(&hasher) | 1;
        Some(Arc::new(Self {
            base,
            runs,
            revision,
            reach,
        }))
    }

    /// The effects governing byte `offset`.
    pub fn effects_at(&self, offset: usize) -> &SceneTextEffects {
        let index = self.runs.partition_point(|run| run.range.end <= offset);
        self.runs
            .get(index)
            .filter(|run| run.range.start <= offset)
            .map_or(&self.base, |run| &run.effects)
    }

    fn all(&self) -> impl Iterator<Item = &SceneTextEffects> {
        std::iter::once(&self.base).chain(self.runs.iter().map(|run| &run.effects))
    }

    /// The most shadow layers any range draws.
    pub fn max_shadows(&self) -> usize {
        self.all()
            .map(|effects| effects.shadows.len())
            .max()
            .unwrap_or(0)
    }

    pub fn has_stroke(&self) -> bool {
        self.all().any(|effects| effects.stroke.is_some())
    }

    pub fn has_decoration(&self) -> bool {
        self.all().any(|effects| effects.decoration.is_active())
    }

    /// A node's effects from its resolved CSS, with `rich`'s paint tier over
    /// them.
    pub fn from_style(
        style: &nana_ui_core::LayoutStyle,
        rich: Option<&nana_ui_core::RichText>,
    ) -> Option<Arc<Self>> {
        let base = css_text_effects(style);
        let runs = rich
            .map(|rich| {
                rich.spans()
                    .iter()
                    .filter(|(_, span)| {
                        let paint = &span.paint;
                        paint.decoration.is_some()
                            || paint.decoration_color.is_some()
                            || paint.stroke.is_some()
                            || paint.shadows.is_some()
                    })
                    .map(|(range, span)| SceneRichRun {
                        range,
                        effects: base.overlay(&span.paint),
                    })
                    .collect()
            })
            .unwrap_or_default();
        Self::new(base, runs)
    }
}

fn hash_paint(color: &nana_ui_core::PaintColor, hasher: &mut impl std::hash::Hasher) {
    let (channels, alpha) = color.to_linear_sc_rgb();
    for value in [channels[0], channels[1], channels[2], alpha] {
        hasher.write_u32(value.to_bits());
    }
}

fn hash_effects(effects: &SceneTextEffects, hasher: &mut impl std::hash::Hasher) {
    hasher.write_u8(
        u8::from(effects.decoration.underline) | u8::from(effects.decoration.line_through) << 1,
    );
    if let Some(color) = &effects.decoration_color {
        hash_paint(color, hasher);
    }
    hasher.write_u8(0xfe);
    if let Some(stroke) = &effects.stroke {
        hasher.write_u32(stroke.width_px.to_bits());
        hash_paint(&stroke.color, hasher);
        hasher.write_u8(stroke.join as u8);
        hasher.write_u8(stroke.placement as u8);
    }
    hasher.write_u8(0xfd);
    for shadow in effects.shadows.iter() {
        for value in [
            shadow.offset[0],
            shadow.offset[1],
            shadow.blur_px,
            shadow.spread_px,
        ] {
            hasher.write_u32(value.to_bits());
        }
        hash_paint(&shadow.color, hasher);
    }
    hasher.write_usize(effects.shadows.len());
}

/// A node's text effects as its CSS states them: `text-decoration`, every
/// `text-shadow` layer, `-webkit-text-stroke` under `paint-order`.
pub fn css_text_effects(style: &nana_ui_core::LayoutStyle) -> SceneTextEffects {
    let paint_of = |paint: Option<nana_ui_core::PaintColor>, srgb: [f32; 4]| {
        paint.unwrap_or(nana_ui_core::PaintColor::srgb(srgb))
    };
    let layers: Vec<nana_ui_core::TextShadowSpec> = if style.paint.text_shadows.is_empty() {
        style.paint.text_shadow.into_iter().collect()
    } else {
        style.paint.text_shadows.clone()
    };
    let shadows: Arc<[nana_ui_core::RichTextShadow]> = layers
        .iter()
        .take(nana_ui_core::MAX_TEXT_SHADOWS)
        .map(|shadow| nana_ui_core::RichTextShadow {
            offset: [shadow.offset_x, shadow.offset_y],
            blur_px: shadow.blur_radius.max(0.0),
            spread_px: 0.0,
            color: paint_of(shadow.paint_color, shadow.color),
        })
        .collect();
    let fill = style
        .paint_colors
        .color
        .or(style.color.map(nana_ui_core::PaintColor::srgb));
    let stroke = style
        .paint
        .text_stroke
        .filter(|stroke| stroke.width > 0.0 && stroke.width.is_finite())
        .and_then(|stroke| {
            let color = stroke
                .paint_color
                .or(stroke.color.map(nana_ui_core::PaintColor::srgb))
                .or(fill)?;
            Some(nana_ui_core::RichTextStroke {
                width_px: stroke.width,
                color,
                join: nana_ui_core::TextStrokeJoin::Miter,
                placement: if style.paint.paint_order_stroke_first {
                    nana_ui_core::TextStrokePlacement::Under
                } else {
                    nana_ui_core::TextStrokePlacement::Over
                },
            })
        });
    SceneTextEffects {
        decoration: style.text_decoration.unwrap_or_default(),
        decoration_color: None,
        stroke,
        shadows,
    }
}

/// The page width a retained layout is mapped into and the y its lines start
/// at inside `bounds`: the same vertical alignment the painter applies.
fn text_frame(
    bounds: SceneRect,
    vertical: TextVerticalAlignment,
    retained: &nana_ui_runtime::RetainedTextLayout,
) -> (f32, f32) {
    let layout = &retained.layout;
    let (box_width, laid_out_height) = layout.physical_size();
    let top = if layout.is_vertical() {
        bounds.y
    } else {
        match vertical {
            TextVerticalAlignment::Top => bounds.y,
            TextVerticalAlignment::Center => bounds.y + (bounds.height - laid_out_height) * 0.5,
            TextVerticalAlignment::Bottom => bounds.y + bounds.height - laid_out_height,
        }
    };
    (box_width.max(bounds.width), top)
}

/// A rich text editor's selection (under the glyphs) and caret (over them),
/// from the very layout its text is drawn from.
#[allow(clippy::too_many_arguments)]
fn rich_editor_mark_primitives(
    context: &VisualPrimitiveContext<'_>,
    bounds: SceneRect,
    vertical: TextVerticalAlignment,
    retained: &nana_ui_runtime::RetainedTextLayout,
    marks: &nana_ui_runtime::RichEditorMarks,
    selection_color: [f32; 4],
    caret_color: [f32; 4],
) -> (Vec<ScenePrimitive>, Option<ScenePrimitive>) {
    let layout = &retained.layout;
    let (page_width, top) = text_frame(bounds, vertical, retained);
    let selection = if marks.selection.is_empty() {
        Vec::new()
    } else {
        layout
            .selection_rects(marks.selection.clone())
            .into_iter()
            .enumerate()
            .map(|(index, rect)| {
                let rect = layout.page_rect(rect, page_width);
                visual_quad(
                    context,
                    collection_slot(DOCUMENT_TEXT_SELECTION, RICH_EDITOR_SELECTION_BASE + index),
                    SceneRect {
                        x: bounds.x + rect.x,
                        y: top + rect.y,
                        width: rect.width,
                        height: rect.height,
                    },
                    VisualQuadStyle::solid(selection_color),
                )
            })
            .collect()
    };
    let caret = marks.caret.and_then(|byte| {
        let (x, line_top, height) = nana_ui_runtime::rich_editor_caret_box(layout, byte)?;
        Some(visual_quad(
            context,
            collection_slot(TEXT_EDITOR_CARET, 0),
            SceneRect {
                x: bounds.x + x - 0.5,
                y: top + line_top,
                width: 1.5,
                height,
            },
            VisualQuadStyle::solid(caret_color),
        ))
    });
    (selection, caret)
}

/// An inline object a presenting text node carries, as it was laid out.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct GlyphObject {
    pub slot: u64,
    pub ordinal: u32,
    pub center: [f32; 2],
    pub opacity: f32,
    pub transform: AffineTransform,
}

/// One primitive per inline object `layout` placed, in text order, where the
/// text primitive at `bounds` draws its lines.
///
/// An image is a quad sampling its `url()`; a host texture slot is the host
/// texture renderer's custom node; an editor chip is drawn only when
/// `editor` asks for it (and takes no room either way). Objects keep the
/// node's transform, clips, opacity and document order, so they scroll, clip
/// and fade with their text.
fn inline_object_primitives(
    context: &VisualPrimitiveContext<'_>,
    bounds: SceneRect,
    vertical: TextVerticalAlignment,
    retained: &nana_ui_runtime::RetainedTextLayout,
    rich: &nana_ui_core::RichText,
    editor: bool,
) -> Vec<ScenePrimitive> {
    let layout = &retained.layout;
    let (box_width, top) = text_frame(bounds, vertical, retained);
    let mut out = Vec::new();
    for (index, placed) in layout.objects.iter().enumerate() {
        let Some(object) = rich.object_at(placed.offset) else {
            continue;
        };
        let rect = layout.page_rect(placed.rect, box_width.max(bounds.width));
        let object_bounds = SceneRect {
            x: bounds.x + rect.x,
            y: top + rect.y,
            width: rect.width,
            height: rect.height,
        };
        let kind = match &object.content {
            nana_ui_core::RichObjectContent::Image { source } => {
                if rect.width <= 0.0 || rect.height <= 0.0 {
                    continue;
                }
                custom_paint::image_quad(
                    source,
                    nana_ui_runtime::ImageFit::Contain,
                    nana_ui_core::ImageSampling::default(),
                    [0.0; 4],
                )
            }
            nana_ui_core::RichObjectContent::HostTexture { slot } => {
                if rect.width <= 0.0 || rect.height <= 0.0 {
                    continue;
                }
                ScenePrimitiveKind::Custom {
                    node: nana_ui_runtime::CustomRenderNode {
                        fit: nana_ui_core::ContentFit::Contain,
                        ..nana_ui_runtime::CustomRenderNode::new(
                            nana_ui_runtime::HOST_TEXTURE_RENDERER,
                            Arc::clone(slot),
                            0,
                        )
                    },
                    mask: None,
                    corner_radius: [0.0; 4],
                }
            }
            nana_ui_core::RichObjectContent::Tag { kind, .. } => {
                let Some(label) = layout
                    .labels
                    .iter()
                    .find(|label| label.offset == placed.offset)
                else {
                    continue;
                };
                let pill = layout.page_rect(label.rect, box_width.max(bounds.width));
                let pill = SceneRect {
                    x: bounds.x + pill.x,
                    y: top + pill.y,
                    width: pill.width,
                    height: pill.height,
                };
                let mut style = VisualQuadStyle::solid(tag_color(*kind));
                style.corner_radius = corner_radii(pill.height * 0.5);
                out.push(visual_quad(
                    context,
                    collection_slot(DOCUMENT_TEXT_SELECTION, RICH_TAG_BASE + index),
                    pill,
                    style,
                ));
                continue;
            }
            nana_ui_core::RichObjectContent::Chip { .. } => {
                if !editor {
                    continue;
                }
                // The marker takes no room; the editor shows it as a thin
                // caret-high bar where it sits.
                let height = layout
                    .lines
                    .get(placed.line as usize)
                    .map_or(bounds.height, |line| line.metrics.height_px);
                let line_top = layout
                    .lines
                    .get(placed.line as usize)
                    .map_or(0.0, |line| line.metrics.top_y_px);
                let marker = SceneRect {
                    x: object_bounds.x - 1.0,
                    y: top + line_top,
                    width: 2.0,
                    height,
                };
                out.push(visual_quad(
                    context,
                    collection_slot(TEXT_INLINE_OBJECTS, index),
                    marker,
                    VisualQuadStyle::solid([0.95, 0.6, 0.2, 0.9]),
                ));
                continue;
            }
        };
        out.push(ScenePrimitive {
            id: PrimitiveId {
                node: context.node,
                slot: collection_slot(TEXT_INLINE_OBJECTS, index),
            },
            node: context.node,
            bounds: object_bounds,
            transform: context.transform,
            clips: Arc::clone(context.clips),
            opacity: context.opacity,
            z_index: context.z_index,
            document_order: context.document_order,
            kind,
        });
    }
    out
}

/// Fill colours of `rich`'s spans as scene text spans, cut around the spans
/// already there (selection and syntax colours win over a span's own fill).
pub fn rich_fill_spans(
    content: &str,
    rich: Option<&nana_ui_core::RichText>,
    existing: Vec<SceneTextSpan>,
) -> Vec<SceneTextSpan> {
    let Some(rich) = rich.filter(|rich| rich.text() == content) else {
        return existing;
    };
    let mut taken: Vec<(usize, usize)> =
        existing.iter().map(|span| (span.start, span.end)).collect();
    taken.sort_unstable();
    let mut out = existing;
    for (range, span) in rich.spans().iter() {
        let Some(color) = span.paint.color else {
            continue;
        };
        let mut cursor = range.start;
        for &(start, end) in taken
            .iter()
            .filter(|(start, end)| *end > range.start && *start < range.end)
        {
            if cursor < start {
                out.push(SceneTextSpan {
                    start: cursor,
                    end: start.min(range.end),
                    color,
                });
            }
            cursor = cursor.max(end);
        }
        if cursor < range.end {
            out.push(SceneTextSpan {
                start: cursor,
                end: range.end,
                color,
            });
        }
    }
    out.sort_by_key(|span| (span.start, span.end));
    out
}

/// OpenType and wrap extras on a [`ScenePrimitiveKind::Text`] run.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct SceneTextOpenType {
    pub features: Vec<FontFeatureSetting>,
    pub variations: Vec<FontVariationSetting>,
    pub kerning: FontKerningSpec,
    pub word_break: WordBreakSpec,
    pub line_break: LineBreakSpec,
    /// CSS `direction` after inherit. Drives the same RLI/PDI wrap as shaping.
    pub direction: DirSpec,
    /// Whether an authored newline is a line break rather than a space.
    ///
    /// CSS `white-space` decides it, and a multiline editor's value keeps its
    /// breaks whatever `white-space` says. The renderer cannot derive it from
    /// anything else here, and getting it wrong measures one line and paints
    /// two.
    pub preserve_lines: bool,
    /// CSS `writing-mode` after inherit. Vertical text — an editor's value
    /// included — is laid out and painted in columns (#59).
    pub writing_mode: WritingModeSpec,
    /// CSS `text-orientation` after inherit (#59).
    pub text_orientation: nana_ui_core::TextOrientationSpec,
    /// The language the Runtime shaped this text in (BCP 47). A renderer that
    /// lays the text out again shapes in the same one, or its `locl` forms
    /// and fallback faces would differ from what was measured.
    pub language: Option<nana_ui_runtime::LanguageTag>,
}

impl SceneTextOpenType {
    pub fn from_computed(style: &nana_ui_runtime::ComputedStyle) -> Self {
        Self {
            features: style.font_features.clone(),
            variations: style.font_variations.clone(),
            kerning: style.font_kerning,
            word_break: style.word_break,
            line_break: style.line_break,
            // The used direction: `text-orientation: upright` reads `ltr`.
            direction: style.writing_context().direction,
            writing_mode: style.writing_mode,
            text_orientation: style.text_orientation,
            language: style.language.clone(),
            // Not on `ComputedStyle`: `white-space` is a box-layout property,
            // so the caller that has the layout style sets it.
            preserve_lines: false,
        }
    }
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

/// Isolating ancestor whose subtree is composited as one layer.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct OpacityGroup {
    pub node: StableNodeId,
    pub opacity: f32,
    pub filter: ColorFilter,
    pub mix_blend: MixBlendMode,
    /// Inset `box-shadow` overlay recorded on this dest group. `None` when none.
    pub inset_shadow: Option<InsetShadowOverlay>,
}

/// Inset shadow painted onto a dest group after its descendants composite.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct InsetShadowOverlay {
    pub elevation: ComponentElevation,
    pub bounds: SceneRect,
    pub corner_radius: [f32; 4],
}

/// Isolating ancestor with a non-identity CSS `filter`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FilterGroup {
    pub node: StableNodeId,
    pub filter: ColorFilter,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct SceneOrderKey {
    /// `(z_index, document_order)` for each isolating stacking group from
    /// outermost to innermost, then this primitive's node. Opacity groups,
    /// `isolation: isolate`, and positioned + `z-index` keep a subtree
    /// contiguous against siblings, except `position: fixed` surfaces which
    /// paint in the root stacking context so Popover/menu chrome is not trapped
    /// in a parent card. Not full CSS Appendix E.
    stack: GroupPrefix,
    /// Collection identity must not lift scrolling text above sticky bands,
    /// minimaps or popup surfaces owned by the same component.
    paint_layer: u64,
    slot: u64,
    node: StableNodeId,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SceneDelta {
    pub added: Vec<StableNodeId>,
    pub removed: Vec<StableNodeId>,
    pub paint: Vec<StableNodeId>,
    pub transforms: Vec<StableNodeId>,
    pub clips: Vec<StableNodeId>,
    pub order_changed: bool,
    pub stats: SceneDeltaStats,
}

impl std::ops::Deref for SceneDelta {
    type Target = SceneDeltaStats;

    fn deref(&self) -> &Self::Target {
        &self.stats
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SceneDeltaStats {
    pub updated_nodes: usize,
    pub removed_nodes: usize,
    pub rebuilt_primitives: usize,
    pub order_rebuilt: bool,
    pub primitive_count: usize,
}

/// A primitive plus what the scene needs to keep it in paint order.
#[derive(Debug, Clone)]
struct RetainedPrimitive {
    primitive: ScenePrimitive,
    /// Where this primitive sits in `ordered`. Kept rather than recomputed:
    /// the walk that produces it is the expensive half of touching a
    /// primitive, and removal has to use the key it was filed under.
    key: SceneOrderKey,
    /// The rebuild that last wrote this slot, against the scene's `build`
    /// counter. A slot an ongoing rebuild has not written is one the node no
    /// longer has.
    build: u64,
}

#[derive(Debug)]
pub struct UiScene {
    frame_plan: OnceLock<Arc<FramePlan>>,
    visibility: OnceLock<VisibilityIndex>,
    attribute_epoch: u64,
    projections: NodeMap<(u64, AffineTransform, usize)>,
    /// Projections that cannot be adjusted by an inverse delta. Usually empty;
    /// ordinary scrolling must not scan every retained descendant.
    unadjustable_projections: NodeSet,
    draw_attributes: std::sync::Mutex<NodeMap<DrawAttributes>>,
    /// Dest groups per node, stamped with the scene instance they were read
    /// at. See [`UiScene::opacity_groups`].
    opacity_group_cache: std::sync::Mutex<OpacityGroupCache>,
    /// Ancestor compositor-layer opacity factors, stamped with the scene
    /// instance and the attribute epoch. See
    /// [`UiScene::compositor_paint_opacity`].
    pub(super) layer_factor_cache: std::sync::Mutex<LayerFactorCache>,
    /// What the primitive-rebuild pass would otherwise recompute for every
    /// node it touches. See [`RebuildScratch`].
    rebuild_scratch: std::sync::Mutex<RebuildScratch>,
    nodes: SceneNodes,
    node_order: NodeMap<usize>,
    primitives: BTreeMap<PrimitiveId, RetainedPrimitive>,
    ordered: BTreeSet<SceneOrderKey>,
    /// Bumped once per node rebuild and stamped onto every primitive that
    /// rebuild writes. See `rebuild_node_primitives`.
    build: u64,
    /// Whether this delta added, dropped or re-bound a primitive — the changes
    /// a compiled frame plan and the culling index are built on. The scene
    /// knows it as it happens: this is the pass that adds and drops them.
    /// Asking afterwards meant snapshotting every touched node's primitive
    /// list and diffing it, which is a range scan and an allocation per node.
    structure_changed: bool,
    /// An extraction changed retained geometry while compositor layers may
    /// still be presenting the previous projection. The compositor pass
    /// clears this after rebasing the visibility index for the current layer.
    compositor_projection_dirty: bool,
    compositor: CompositorRegistry,
    /// Geometry built from each custom-painted node's recording, reused while
    /// the runtime hands back the same recording. Empty unless a node has a
    /// `Painter`.
    custom_paint: NodeMap<custom_paint::BuiltPaint>,
    /// Nodes that [`may_be_dest_group`] admits. Paint asks
    /// [`UiScene::opacity_groups`] once per primitive, and even the memoized
    /// walk behind it has to read a cold `ExtractedNode` and its style for the
    /// queried node itself; a zero here proves no walk can find a group, so no
    /// frame of a scene without isolation pays for it at all.
    dest_group_candidates: usize,
    /// Text nodes presenting per glyph, and until when what they draw keeps
    /// changing (`None`: a looping effect). What keeps frames coming while a
    /// reveal plays and stops them once it has finished.
    glyph_live: NodeMap<Option<std::time::Duration>>,
    /// Charts in motion, by node: until when their marks or emphasis move.
    chart_live: NodeMap<std::time::Duration>,
    /// Inline objects of presenting text nodes, as laid out, so each
    /// compositor tick can move, scale and fade them the way the glyphs
    /// around them are (the text shader cannot reach them: they are images).
    glyph_objects: NodeMap<(Arc<SceneGlyphPresentation>, Arc<[GlyphObject]>)>,
    /// Identity that changes on node-changing mutation and on Clone.
    /// In-place [`UiScene::apply_delta`] that updates or removes nodes also
    /// gets a fresh value, because product flush mutates a unique `Arc` in
    /// place after the first paint. Painters key a validated op stream on
    /// this id. Never zero: two freshly created scenes must not share an
    /// identity.
    instance: u64,
    /// How every rounded corner of this scene is shaped: the installed
    /// theme's, set by the document that extracts into the scene. See
    /// [`UiScene::corner_shape`].
    corner_shape: nana_ui_core::CornerShape,
}

impl Default for UiScene {
    fn default() -> Self {
        Self {
            frame_plan: OnceLock::new(),
            visibility: OnceLock::new(),
            attribute_epoch: 0,
            projections: NodeMap::default(),
            unadjustable_projections: NodeSet::default(),
            draw_attributes: std::sync::Mutex::new(NodeMap::default()),
            opacity_group_cache: std::sync::Mutex::new(OpacityGroupCache::default()),
            layer_factor_cache: std::sync::Mutex::new(LayerFactorCache::default()),
            rebuild_scratch: std::sync::Mutex::new(RebuildScratch::default()),
            nodes: SceneNodes::default(),
            node_order: NodeMap::default(),
            primitives: BTreeMap::new(),
            ordered: BTreeSet::new(),
            build: next_primitive_revision(),
            structure_changed: false,
            compositor_projection_dirty: false,
            compositor: CompositorRegistry::default(),
            custom_paint: NodeMap::default(),
            dest_group_candidates: 0,
            glyph_live: NodeMap::default(),
            chart_live: NodeMap::default(),
            glyph_objects: NodeMap::default(),
            instance: next_scene_instance(),
            corner_shape: nana_ui_core::CornerShape::Round,
        }
    }
}

impl Clone for UiScene {
    fn clone(&self) -> Self {
        Self {
            frame_plan: self.frame_plan.clone(),
            visibility: self.visibility.clone(),
            attribute_epoch: self.attribute_epoch,
            projections: self.projections.clone(),
            unadjustable_projections: self.unadjustable_projections.clone(),
            draw_attributes: std::sync::Mutex::new(
                self.draw_attributes
                    .lock()
                    .expect("scene attributes")
                    .clone(),
            ),
            // Stamped with the instance they were read at, and a clone is a
            // new instance, so carrying them over would only be work.
            opacity_group_cache: std::sync::Mutex::new(OpacityGroupCache::default()),
            layer_factor_cache: std::sync::Mutex::new(LayerFactorCache::default()),
            rebuild_scratch: std::sync::Mutex::new(RebuildScratch::default()),
            nodes: self.nodes.clone(),
            node_order: self.node_order.clone(),
            primitives: self.primitives.clone(),
            ordered: self.ordered.clone(),
            build: self.build,
            structure_changed: self.structure_changed,
            compositor_projection_dirty: self.compositor_projection_dirty,
            compositor: self.compositor.clone(),
            custom_paint: self.custom_paint.clone(),
            dest_group_candidates: self.dest_group_candidates,
            glyph_live: self.glyph_live.clone(),
            chart_live: self.chart_live.clone(),
            glyph_objects: self.glyph_objects.clone(),
            instance: next_scene_instance(),
            corner_shape: self.corner_shape,
        }
    }
}

/// Identity for one write of a primitive, unique across every scene in the
/// process. See [`UiScene::build`].
pub(super) fn next_primitive_revision() -> u64 {
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
    NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
}

fn next_scene_instance() -> u64 {
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
    NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
}

impl UiScene {
    pub fn new() -> Self {
        Self::default()
    }

    /// Mutation-unique identity. Clone and in-place node-changing
    /// [`Self::apply_delta`] both get a fresh value; idle `apply_delta([], [])`
    /// keeps it so unchanged-scene paint caches still hit.
    pub fn instance_id(&self) -> u64 {
        self.instance
    }

    /// What an answer derived from this scene's projections, transforms and
    /// clip chains stays valid for.
    ///
    /// `instance` moves whenever the scene's nodes do, `attribute_epoch`
    /// whenever what a node inherits does — including a compositor
    /// presentation tick, which moves geometry without touching the node set.
    /// An idle flush moves neither, which is what lets a host mirroring this
    /// geometry into a platform compositor skip the scan entirely rather than
    /// rebuilding the same answer every frame.
    pub const fn projection_revision(&self) -> (u64, u64) {
        (self.instance, self.attribute_epoch)
    }

    /// How every rounded corner in this scene is shaped. Primitives carry
    /// only their radii; a painter reads the shape once per frame.
    pub const fn corner_shape(&self) -> nana_ui_core::CornerShape {
        self.corner_shape
    }

    /// Shape every rounded corner. The document that extracts into the scene
    /// sets its installed theme's shape here. A change is a new
    /// [`Self::instance_id`], so a painter does not reuse a frame it painted
    /// with the old shape.
    pub fn set_corner_shape(&mut self, shape: nana_ui_core::CornerShape) {
        if self.corner_shape != shape {
            self.corner_shape = shape;
            self.instance = next_scene_instance();
        }
    }

    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty()
    }

    pub fn node_count(&self) -> usize {
        self.nodes.len()
    }

    pub fn primitive_count(&self) -> usize {
        self.primitives.len()
    }

    pub fn primitives(&self) -> impl Iterator<Item = &ScenePrimitive> {
        self.ordered.iter().filter_map(|key| {
            self.primitives
                .get(&PrimitiveId {
                    node: key.node,
                    slot: key.slot,
                })
                .map(|held| &held.primitive)
        })
    }

    pub fn node_bounds(&self, id: StableNodeId) -> Option<SceneRect> {
        self.nodes.get(&id).map(|node| SceneRect {
            x: node.layout.x,
            y: node.layout.y,
            width: node.layout.width,
            height: node.layout.height,
        })
    }

    /// Isolating opacity groups from outermost to innermost that contain `node`.
    /// Dest groups containing `node`, outermost first.
    ///
    /// Asked once per primitive per frame, and again by culling, so the
    /// *ancestor* half of the answer is memoized: a container's list is its
    /// parent's list plus itself when it is a group. The queried node's own
    /// list is not stored — every primitive names a different node, so an
    /// entry for it would be written once and read never.
    ///
    /// A node that is not a group **shares** its parent's list rather than
    /// copying it, so a shell whose containers are all opaque allocates
    /// nothing here at all.
    ///
    /// The stamp is the scene instance, which `apply_delta` moves whenever a
    /// node, a style or a parent changed — the only three things this reads.
    pub fn opacity_groups(&self, node: StableNodeId) -> Arc<[OpacityGroup]> {
        if self.dest_group_candidates == 0 {
            return empty_opacity_groups();
        }
        let Some(candidate) = self.nodes.get(&node) else {
            return empty_opacity_groups();
        };
        let base = match candidate.parent {
            Some(parent) => {
                let mut cache = self
                    .opacity_group_cache
                    .lock()
                    .expect("scene opacity groups");
                self.ancestor_opacity_groups(&mut cache, parent, 0)
            }
            None => empty_opacity_groups(),
        };
        if is_dest_group(&self.nodes, candidate) {
            let mut groups = base.to_vec();
            groups.push(dest_group(&self.nodes, node, candidate));
            return Arc::from(groups);
        }
        base
    }

    fn ancestor_opacity_groups(
        &self,
        cache: &mut OpacityGroupCache,
        id: StableNodeId,
        depth: usize,
    ) -> Arc<[OpacityGroup]> {
        if depth >= MAX_ANCESTOR_DEPTH {
            return empty_opacity_groups();
        }
        if let Some((stamp, groups)) = cache.get(&id)
            && *stamp == self.instance
        {
            return Arc::clone(groups);
        }
        let Some(candidate) = self.nodes.get(&id) else {
            return empty_opacity_groups();
        };
        let base = match candidate.parent {
            Some(parent) => self.ancestor_opacity_groups(cache, parent, depth + 1),
            None => empty_opacity_groups(),
        };
        let groups = if is_dest_group(&self.nodes, candidate) {
            let mut groups = base.to_vec();
            groups.push(dest_group(&self.nodes, id, candidate));
            Arc::from(groups)
        } else {
            base
        };
        cache.insert(id, (self.instance, Arc::clone(&groups)));
        groups
    }

    /// Isolating filter groups from outermost to innermost that contain `node`.
    pub fn filter_groups(&self, node: StableNodeId) -> Vec<FilterGroup> {
        filter_groups_from(&self.nodes, node)
    }

    pub fn is_node_in_subtree(&self, root: StableNodeId, candidate: StableNodeId) -> bool {
        ancestor_ids(&self.nodes, candidate).any(|id| id == root)
    }

    /// Apply Runtime's dirty extraction and tombstone stream atomically.
    /// Updating or removing nodes refreshes [`Self::instance_id`]; an empty
    /// no-op keeps the current instance.
    pub fn apply_delta(
        &mut self,
        extracted: impl IntoIterator<Item = ExtractedNode>,
        removals: impl IntoIterator<Item = StableNodeId>,
    ) -> SceneDelta {
        let mut delta = SceneDelta::default();
        self.structure_changed = false;
        let mut removed_nodes = 0;
        let mut changed = Vec::new();
        let mut hierarchy_changed = false;
        let mut inherited_roots = HashSet::new();
        let mut custom_paint_changed = false;
        for id in removals {
            if let Some(old) = self.nodes.remove(&id) {
                self.dest_group_candidates -= usize::from(may_be_dest_group(&old));
                delta.removed.push(id);
                self.projections.remove(&id);
                self.unadjustable_projections.remove(&id);
                if !old.children.is_empty() {
                    self.attribute_epoch = self.attribute_epoch.wrapping_add(1);
                    inherited_roots.insert(id);
                }
                self.draw_attributes
                    .get_mut()
                    .expect("scene attributes")
                    .remove(&id);
                removed_nodes += 1;
                hierarchy_changed |= old.parent.is_some() || !old.children.is_empty();
                self.remove_node_primitives(id);
                self.forget_compositor_node(id);
                if !self.custom_paint.is_empty() {
                    self.custom_paint.remove(&id);
                }
            }
        }
        let mut updated_nodes = 0;
        // Roots whose retained descendants have to be rebuilt because what
        // they inherit changed. A scrolled projective subtree cannot be
        // re-projected, and a group that stops isolating hands its opacity
        // down to primitives that baked it.
        let mut subtree_rebuild = Vec::new();
        let mut scroll_translations = Vec::new();
        let mut stacking_changed = false;
        let mut inherited_geometry_changed = !inherited_roots.is_empty();
        for node in extracted {
            let previous = self.nodes.get(&node.id);
            // Gaining or losing a painter changes which primitives a node
            // owns without necessarily touching the frame plan, so the
            // retained index's slots for it cannot be trusted. A new
            // recording on the same painter is ordinary: the node is rebuilt
            // below and `VisibilityIndex::update` re-derives its slots from
            // the new primitives, keeping the index for everything else.
            custom_paint_changed |= previous
                .is_some_and(|old| old.custom_paint.is_some() != node.custom_paint.is_some());
            let inherited_changed = previous.map_or(!node.children.is_empty(), |old| {
                old.parent != node.parent
                    || old.layout != node.layout
                    || inherited_layout_changed(&old.source_style.layout, &node.source_style.layout)
                    || inherited_component_clip(old) != inherited_component_clip(&node)
            });
            if inherited_changed {
                // Retained descendants may not be extracted when an ancestor's
                // clip or transform changes. Refresh their inherited projection
                // without rebuilding invertible retained geometry.
                self.attribute_epoch = self.attribute_epoch.wrapping_add(1);
                inherited_geometry_changed |=
                    previous.is_none_or(|old| inherited_transform_changed(old, &node));
                inherited_roots.insert(node.id);
            }
            if previous.is_none() {
                delta.added.push(node.id);
            }
            if previous.is_none_or(|old| {
                old.layout != node.layout || old.scroll_offset != node.scroll_offset
            }) {
                delta.transforms.push(node.id);
            }
            // Layout/style can affect inherited clips; consumers resolve the
            // changed attribute roots rather than guessing from paint counts.
            if previous.is_none_or(|old| {
                old.parent != node.parent
                    || old.layout != node.layout
                    || inherited_layout_changed(&old.source_style.layout, &node.source_style.layout)
                    || inherited_component_clip(old) != inherited_component_clip(&node)
            }) {
                delta.clips.push(node.id);
            }
            delta.paint.push(node.id);
            hierarchy_changed |= previous.is_none_or(|old| {
                old.parent != node.parent
                    || old.children != node.children
                    || old.source_style.layout.position != node.source_style.layout.position
            });
            let scroll_changed =
                previous.is_some_and(|old| old.scroll_offset != node.scroll_offset);
            // A node's z_index and whether it opens a group are part of every
            // descendant's paint-order key, and descendants do not have to be
            // re-extracted with it, so such a change costs a full reorder.
            stacking_changed |=
                previous.is_some_and(|old| paint_order_facts(old) != paint_order_facts(&node));
            // Of what a node hands down, its opacity is the one a descendant
            // bakes into its primitive rather than re-deriving at draw time. A
            // group isolates it, so the number that reaches descendants is 1.0
            // until the node stops being a group.
            if previous.is_some_and(|old| {
                inherited_opacity(&self.nodes, old).to_bits()
                    != inherited_opacity(&self.nodes, &node).to_bits()
            }) {
                subtree_rebuild.push(node.id);
            }
            changed.push(node.id);
            if scroll_changed {
                inherited_roots.insert(node.id);
                let old = self.nodes.get(&node.id).expect("scrolling retained node");
                let (parent, _, _, blocks_3d) = self.ancestor_state(old);
                let transform = parent.then(node_scene_transform(
                    &old.source_style.layout,
                    old.layout,
                    blocks_3d,
                ));
                if transform.is_projective() {
                    subtree_rebuild.push(node.id);
                    self.visibility.take();
                } else {
                    let dx = old.scroll_offset.x - node.scroll_offset.x;
                    let dy = old.scroll_offset.y - node.scroll_offset.y;
                    let [a, b, c, d, _, _] = transform.0;
                    scroll_translations.push((node.id, [a * dx + c * dy, b * dx + d * dy]));
                }
                self.attribute_epoch = self.attribute_epoch.wrapping_add(1);
            }
            // The primitives stay where they are: every extracted node is
            // rebuilt below, and that rebuild overwrites the slots it still
            // has and retires the ones it does not. Dropping them here would
            // only mean taking each one out of `ordered` and putting it back.
            self.retain_compositor_requests(&node);
            let candidate = may_be_dest_group(&node);
            let replaced = self.nodes.insert(node.id, Arc::new(node));
            self.dest_group_candidates -=
                usize::from(replaced.as_deref().is_some_and(may_be_dest_group));
            self.dest_group_candidates += usize::from(candidate);
            updated_nodes += 1;
        }
        let order_rebuilt = (updated_nodes != 0 || removed_nodes != 0)
            && (hierarchy_changed || self.node_order.len() != self.nodes.len());
        let mut rebuilt_primitives = 0;
        if updated_nodes != 0 || removed_nodes != 0 {
            self.compositor_projection_dirty =
                self.compositor_layer_count() != 0 && scroll_translations.is_empty();
            if order_rebuilt {
                self.rebuild_document_order();
                for held in self.primitives.values_mut() {
                    if let Some(order) = self.node_order.get(&held.primitive.node) {
                        held.primitive.document_order = *order;
                    }
                }
            }
            let mut rebuild = changed;
            if !subtree_rebuild.is_empty() {
                let extracted: NodeSet = rebuild.iter().copied().collect();
                for root in subtree_rebuild {
                    collect_unextracted_descendants(&self.nodes, root, &extracted, &mut rebuild);
                }
            }
            // An old projective/singular base cannot be mapped to the new
            // inherited transform. Rebuild only those affected projections;
            // invertible siblings and the ordinary scroll fast path stay retained.
            let mut rebuilding: HashSet<_> = rebuild.iter().copied().collect();
            for &id in &self.unadjustable_projections {
                if !rebuilding.contains(&id)
                    && inherited_roots
                        .iter()
                        .any(|root| self.node_descends_from(id, *root))
                {
                    rebuilding.insert(id);
                    rebuild.push(id);
                }
            }
            rebuild.retain(|id| rebuilding.remove(id));
            // `self.nodes` is final by now and nothing below moves it, which
            // is the whole precondition for reusing what a parent projects.
            if let Ok(mut scratch) = self.rebuild_scratch.lock() {
                scratch.begin();
            }
            for &id in &rebuild {
                rebuilt_primitives += self.rebuild_node_primitives(id);
                self.invalidate_compositor_cache(id);
            }
            if let Ok(mut scratch) = self.rebuild_scratch.lock() {
                scratch.end();
            }
            // Rebuilt nodes re-enter `ordered` at their own key, so a reorder is
            // only needed when keys the delta did not touch also moved.
            if order_rebuilt || stacking_changed {
                self.sort_primitives();
            }
            if order_rebuilt || stacking_changed || removed_nodes != 0 || self.structure_changed {
                self.frame_plan.take();
                self.visibility.take();
            }
            if custom_paint_changed {
                self.visibility.take();
            }
            if let Some(mut visibility) = self.visibility.take() {
                for &(root, offset) in &scroll_translations {
                    visibility.translate_subtree(root, offset);
                }
                if inherited_geometry_changed {
                    // An ancestor's projection moved, and its retained
                    // descendants are not in `rebuild`, so their bounds have to
                    // be re-derived — but only under the roots that moved, and
                    // from the plan this index already holds rather than a new
                    // one. Which primitives sit under those roots did not
                    // change, and that is the half rebuilding would pay for
                    // again.
                    for &root in &inherited_roots {
                        visibility.refresh_subtree(self, root);
                    }
                }
                visibility.update(self, &rebuild);
                let _ = self.visibility.set(visibility);
            }
            self.instance = next_scene_instance();
        }
        #[cfg(debug_assertions)]
        self.audit_retained_projection();
        // The counter is what lets `opacity_groups` answer without touching the
        // node map, so a path that edits it without maintaining the counter
        // would drop isolation groups from paint and show nothing else. There
        // are two such paths today; this catches a third being added.
        // Rescanning is linear in the scene, so it is bounded to the small
        // trees unit tests build — a new mutation site will be reached by one
        // of those long before it is reached by a scene big enough for the
        // bound to matter.
        debug_assert!(
            self.nodes.len() > RETAINED_AUDIT_LIMIT
                || self.dest_group_candidates
                    == self
                        .nodes
                        .iter()
                        .filter(|(_, node)| may_be_dest_group(node))
                        .count(),
            "dest_group_candidates drifted from the node map"
        );
        delta.order_changed = order_rebuilt || stacking_changed;
        delta.stats = SceneDeltaStats {
            updated_nodes,
            removed_nodes,
            rebuilt_primitives,
            order_rebuilt,
            primitive_count: self.primitives.len(),
        };
        delta
    }

    /// Assert that whatever [`Self::apply_delta`] chose to keep still equals
    /// what recomputing it would produce.
    ///
    /// Retaining the paint order, the frame plan or the visibility index is a
    /// judgement about which style changes can move them, and getting that
    /// wrong paints the right pixels in the wrong order — the kind of bug no
    /// still-frame snapshot catches, because the frame it is wrong on is the
    /// one *after* a mutation. Running the check inside every delta makes the
    /// whole existing suite a test of the invalidation rules, at O(scene) per
    /// delta, which is why it is bounded to the trees unit tests build.
    ///
    /// An index the scroll fast path has shifted skips the visibility half:
    /// it holds bounds moved by an offset instead of re-derived from layout,
    /// and shifts it has not pushed down to its leaves yet, so it answers a
    /// query the same as a fresh build without matching one bit for bit.
    ///
    /// That is a property of the index, and it lasts until something
    /// re-derives the bounds — not of the delta that did the shifting, which
    /// is why the index carries the flag. Order and plan are still checked.
    #[cfg(debug_assertions)]
    fn audit_retained_projection(&self) {
        if self.nodes.len() > RETAINED_AUDIT_LIMIT {
            return;
        }
        let mut stacks: HashMap<StableNodeId, GroupPrefix> = HashMap::new();
        let fresh: BTreeSet<SceneOrderKey> = self
            .primitives
            .values()
            .map(|held| {
                let primitive = &held.primitive;
                if let Some(key) = triggered_overlay_surface_key(primitive) {
                    return key;
                }
                if !self.custom_paint.is_empty() && has_custom_paint(&self.nodes, primitive.node) {
                    return order_key(&self.nodes, &self.node_order, primitive);
                }
                let stack = stacks.entry(primitive.node).or_insert_with(|| {
                    let prefix: GroupPrefix =
                        group_prefix(&self.nodes, &self.node_order, primitive.node).into();
                    order_stack(&self.nodes, &prefix, primitive)
                });
                SceneOrderKey::at(Arc::clone(stack), primitive)
            })
            .collect();
        assert!(
            self.ordered == fresh,
            "retained paint order disagrees with a fresh sort"
        );
        // Painted geometry lives only for painted nodes, and every painted
        // primitive has the geometry it was cut from (Issue #217).
        assert!(
            self.custom_paint.keys().all(|id| self
                .nodes
                .get(id)
                .is_some_and(|node| node.custom_paint.is_some())),
            "painted geometry held for a node that no longer paints"
        );
        assert!(
            self.primitives.values().all(|held| !matches!(
                held.primitive.kind,
                ScenePrimitiveKind::Path { .. }
                    | ScenePrimitiveKind::LayerBegin { .. }
                    | ScenePrimitiveKind::LayerEnd { .. }
            ) || self
                .custom_paint
                .contains_key(&held.primitive.node)),
            "a painted primitive without its geometry"
        );
        assert!(
            self.primitives
                .values()
                .all(|held| self.ordered.contains(&held.key)),
            "a retained primitive is filed under a key the order does not hold"
        );
        let Some(plan) = self.frame_plan.get() else {
            return;
        };
        match self.build_frame_plan() {
            Ok(fresh) => assert!(
                plan.operations == fresh.operations
                    && plan.preparations == fresh.preparations
                    && plan.custom_nodes == fresh.custom_nodes,
                "retained frame plan disagrees with a fresh build"
            ),
            // A plan that no longer compiles is reported by `frame_plan`, not here.
            Err(_) => return,
        }
        if let Some(visibility) = self.visibility.get() {
            if visibility.translated() {
                return;
            }
            let fresh = VisibilityIndex::new(self, Arc::clone(plan));
            if let Some(mismatch) = visibility.mismatch(&fresh) {
                panic!("retained visibility index disagrees with a fresh build: {mismatch}");
            }
        }
    }

    /// What a frame plan reads out of a primitive beyond its identity.
    fn custom_binding(kind: &ScenePrimitiveKind) -> Option<(&Arc<str>, &Arc<str>)> {
        match kind {
            ScenePrimitiveKind::Custom { node, .. } => Some((&node.renderer, &node.resource)),
            _ => None,
        }
    }

    pub fn primitive(&self, id: PrimitiveId) -> Option<&ScenePrimitive> {
        self.primitives.get(&id).map(|held| &held.primitive)
    }

    /// The primitive and the rebuild that last wrote it. See
    /// [`SceneDraw::revision`].
    fn primitive_at(&self, id: PrimitiveId) -> Option<(&ScenePrimitive, u64)> {
        self.primitives
            .get(&id)
            .map(|held| (&held.primitive, held.build))
    }

    /// Rewrite one primitive kind and bump instance identity.
    ///
    /// Painter tests use this to probe stroke variants without a second
    /// Runtime extraction ABI.
    pub fn replace_primitive_kind(&mut self, id: PrimitiveId, kind: ScenePrimitiveKind) -> bool {
        self.build = next_primitive_revision();
        let build = self.build;
        let Some(held) = self.primitives.get_mut(&id) else {
            return false;
        };
        held.primitive.kind = kind;
        // Anyone keeping a resolved copy of this primitive keys it on the
        // rebuild that wrote it, and this is a write.
        held.build = build;
        self.frame_plan.take();
        self.visibility.take();
        self.instance = next_scene_instance();
        true
    }

    /// Skip duplicate Vue `#text` children when the host already paints that
    /// string. Independent element children, such as list rows inside a Card,
    /// keep their own labels.
    fn parent_already_paints_text(&self, node: &ExtractedNode) -> bool {
        if !matches!(&*node.kind, NodeKind::Text) {
            return false;
        }
        let Some(parent) = node.parent.and_then(|id| self.nodes.get(&id)) else {
            return false;
        };
        if let Some(ComponentGeometry::Card { title, .. }) = parent.component_geometry.as_deref() {
            return title.as_ref().is_some_and(|title| {
                node.text
                    .as_ref()
                    .is_some_and(|text| text.value == title.content.as_ref())
            });
        }
        component_geometry_owns_text(parent.component_geometry.as_deref())
            || parent
                .text
                .as_ref()
                .is_some_and(|text| !text.value.is_empty())
    }

    /// Compact leading glyphs share the parent (or own) text line-box center.
    fn icon_y_aligned_to_adjacent_text(
        &self,
        node: &ExtractedNode,
        icon_bounds: SceneRect,
        extent: f32,
    ) -> f32 {
        let geometric = icon_bounds.y + (icon_bounds.height - extent) / 2.0;
        if node.layout.height > extent + 0.5 || node.layout.width > extent + 0.5 {
            return geometric;
        }
        let host = if node
            .text
            .as_ref()
            .is_some_and(|text| !text.value.is_empty())
        {
            node
        } else {
            match node.parent.and_then(|id| self.nodes.get(&id)) {
                Some(parent)
                    if parent
                        .text
                        .as_ref()
                        .is_some_and(|text| !text.value.is_empty())
                        || matches!(
                            parent.standard_visual.as_ref(),
                            Some(StandardVisual::ListItem { .. })
                        ) =>
                {
                    parent
                }
                _ => return geometric,
            }
        };
        let text_box = match host.component_geometry.as_deref() {
            Some(ComponentGeometry::ListItem {
                content: Some(content),
                ..
            }) => *content,
            _ => host.layout,
        };
        let centered = matches!(
            host.source_style.text_vertical_alignment,
            TextVerticalAlignment::Center
        );
        icon_y_on_text_glyph_center(
            text_box.y,
            text_box.height,
            host.style.font_size,
            host.style.line_height,
            centered,
            extent,
        )
    }
}

fn node_scene_transform(
    style: &nana_ui_core::LayoutStyle,
    layout: LayoutBox,
    block_3d: bool,
) -> AffineTransform {
    if block_3d && style.transform_3d.is_some() {
        return AffineTransform::IDENTITY;
    }
    style
        .world_scene_transform(layout.x, layout.y, layout.width, layout.height)
        .map(|(matrix, persp)| AffineTransform(matrix, persp))
        .unwrap_or_default()
}

impl UiScene {
    fn ancestor_state(
        &self,
        node: &ExtractedNode,
    ) -> (AffineTransform, f32, Arc<[ClipRegion]>, bool) {
        self.project_ancestor_state(node, false)
    }

    fn draw_ancestor_state(
        &self,
        node: &ExtractedNode,
    ) -> (AffineTransform, f32, Arc<[ClipRegion]>, bool) {
        self.project_ancestor_state(node, true)
    }

    fn project_ancestor_state(&self, node: &ExtractedNode, visual: bool) -> AncestorState {
        // What the answer depends on, beyond the chain itself: whether this
        // node breaks out of it, and whether the caller wants the presented
        // values or the logical ones.
        let fixed = node.source_style.layout.position == nana_ui_core::PositionSpec::Fixed;
        let key = node.parent.map(|parent| (parent, fixed, visual));
        if let Some(key) = key
            && let Ok(cache) = self.rebuild_scratch.lock()
        {
            if cache.active {
                if let Some((held, state)) = cache.ancestor_state.as_ref()
                    && *held == key
                {
                    return state.clone();
                }
            } else if let Some((held, stamp, state)) = cache.drawn_ancestor_state.as_ref()
                && *held == key
                && *stamp == self.draw_stamp()
            {
                return state.clone();
            }
        }
        let (state, node_dependent) = self.compute_ancestor_state(node, visual, fixed);
        if let Some(key) = key
            && !node_dependent
            && let Ok(mut cache) = self.rebuild_scratch.lock()
        {
            if cache.active {
                cache.ancestor_state = Some((key, state.clone()));
            } else {
                cache.drawn_ancestor_state = Some((key, self.draw_stamp(), state.clone()));
            }
        }
        state
    }

    /// What a remembered draw-time answer stays valid for.
    ///
    /// `attribute_epoch` moves whenever what a node inherits changes, and
    /// `instance` whenever the scene's nodes do — a leaf leaving does not touch
    /// the epoch but can stop its parent being an opacity group.
    fn draw_stamp(&self) -> DrawStamp {
        self.projection_revision()
    }

    /// The second half of the answer is whether it read anything about `node`
    /// that its siblings do not share — a modal frame above it reads this
    /// node's focus and whether it is in the body, and a workspace resize
    /// handle drops its parent's overflow clip. Neither is cacheable by parent.
    fn compute_ancestor_state(
        &self,
        node: &ExtractedNode,
        visual: bool,
        fixed: bool,
    ) -> (AncestorState, bool) {
        let mut node_dependent = is_workspace_resize_handle(node);
        let mut ancestors = node
            .parent
            .map(|parent| {
                ancestor_nodes(&self.nodes, parent)
                    .map(|(_, node)| node)
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        ancestors.reverse();
        // Layout resolves fixed boxes against the viewport even below a
        // transformed ancestor. Keep structural opacity, but begin geometry at
        // the nearest fixed boundary instead of inheriting its outer scroll,
        // transform and clip chain.
        let geometry_start = if fixed {
            ancestors.len()
        } else {
            ancestors
                .iter()
                .rposition(|ancestor| {
                    ancestor.source_style.layout.position == nana_ui_core::PositionSpec::Fixed
                })
                .unwrap_or(0)
        };
        let mut transform = AffineTransform::IDENTITY;
        let mut opacity = 1.0;
        let mut clips = Vec::new();
        let mut blocks_3d = false;
        for (index, ancestor) in ancestors.into_iter().enumerate() {
            if !is_opacity_group(&self.nodes, ancestor) {
                opacity *= if visual {
                    self.resolved_local_opacity(ancestor)
                } else {
                    local_opacity(ancestor)
                };
            }
            if index < geometry_start {
                continue;
            }
            let layout = ancestor.layout;
            // `blocks_3d` is what the ancestors *above* this one closed, the
            // same question asked of the queried node below — an ancestor's
            // own `perspective` opens a context for its children, not against
            // itself, so it folds in afterwards. Reading it keeps a refusal
            // whole: a node whose `matrix3d` this rule takes away must not go
            // on handing it down, or it paints flat with a rotated inside.
            let local = if visual {
                self.resolved_local_transform(ancestor, blocks_3d)
            } else {
                node_scene_transform(ancestor.source_style.layout.as_ref(), layout, blocks_3d)
            };
            transform = transform.then(local);
            if ancestor.source_style.layout.fails_closed_3d_context() {
                blocks_3d = true;
            }
            if let Some(region) = overflow_clip_region(
                ancestor.source_style.layout.as_ref(),
                SceneRect {
                    x: layout.x,
                    y: layout.y,
                    width: layout.width,
                    height: layout.height,
                },
                transform,
            ) && !(is_workspace_resize_handle(node) && Some(ancestor.id) == node.parent)
            {
                clips.push(region);
            }
            if let Some(ComponentGeometry::EmptyState { root_clip, .. }) =
                ancestor.component_geometry.as_deref()
            {
                clips.push(ClipRegion {
                    bounds: scene_rect(*root_clip),
                    transform,
                    corner_radius: 0.0,
                    polygon_clip: None,
                });
            }
            if let Some(ComponentGeometry::ModalFrame { surface, body, .. }) =
                ancestor.component_geometry.as_deref()
            {
                node_dependent = true;
                let focus_inset = if node.focused { 3.0 } else { 0.0 };
                clips.push(ClipRegion {
                    bounds: SceneRect {
                        x: surface.x - focus_inset,
                        y: surface.y - focus_inset,
                        width: surface.width + focus_inset * 2.0,
                        height: surface.height + focus_inset * 2.0,
                    },
                    transform,
                    corner_radius: 0.0,
                    polygon_clip: None,
                });
                if let Some(StandardVisual::ModalFrame { slots, .. }) =
                    ancestor.standard_visual.as_ref()
                    && slots
                        .body
                        .is_some_and(|body_root| self.node_descends_from(node.id, body_root))
                {
                    clips.push(ClipRegion {
                        bounds: scene_rect(*body),
                        transform,
                        corner_radius: 0.0,
                        polygon_clip: None,
                    });
                }
            }
            let ancestor_bounds = SceneRect {
                x: layout.x,
                y: layout.y,
                width: layout.width,
                height: layout.height,
            };
            if let Some(region) = clip_path_region(
                ancestor.source_style.layout.as_ref(),
                ancestor_bounds,
                transform,
            ) {
                clips.push(region);
            }
            transform = transform.then(AffineTransform::from_matrix([
                1.0,
                0.0,
                0.0,
                1.0,
                -ancestor.scroll_offset.x,
                -ancestor.scroll_offset.y,
            ]));
        }
        (
            (transform, opacity, clips.into(), blocks_3d),
            node_dependent,
        )
    }

    fn remove_node_primitives(&mut self, id: StableNodeId) {
        self.glyph_live.remove(&id);
        self.chart_live.remove(&id);
        self.glyph_objects.remove(&id);
        self.retire_node_primitives(id, |_| true);
    }

    /// Move, scale and fade the inline objects of presenting text nodes to
    /// what the CPU evaluator says their grapheme looks like `now`: the same
    /// arithmetic the text shader applies to the glyphs around them.
    pub(super) fn present_inline_objects(&mut self, now: std::time::Duration) {
        if self.glyph_objects.is_empty() {
            return;
        }
        let mut moved = false;
        for (node, (presentation, objects)) in &self.glyph_objects {
            for object in objects.iter() {
                let sample = nana_ui_core::evaluate_glyph(
                    presentation.effect_of(object.ordinal as usize),
                    presentation.reveal.as_ref(),
                    object.ordinal,
                    now,
                );
                let [cx, cy] = object.center;
                let scale = sample.scale;
                let local = AffineTransform::from_matrix([
                    scale,
                    0.0,
                    0.0,
                    scale,
                    cx + sample.offset[0] - scale * cx,
                    cy + sample.offset[1] - scale * cy,
                ]);
                let Some(held) = self.primitives.get_mut(&PrimitiveId {
                    node: *node,
                    slot: object.slot,
                }) else {
                    continue;
                };
                let opacity = object.opacity * sample.alpha;
                let transform = object.transform.then(local);
                if held.primitive.opacity != opacity || held.primitive.transform != transform {
                    held.primitive.opacity = opacity;
                    held.primitive.transform = transform;
                    moved = true;
                }
            }
        }
        if moved {
            // Painters key what they recorded on the instance.
            self.instance = next_scene_instance();
        }
    }

    /// Whether a text node's per-glyph presentation still changes what is
    /// drawn at the motion clock's time.
    /// Whether a chart's marks or emphasis are still moving: the painter
    /// cannot reuse the painted frame, and the host keeps presenting.
    pub fn chart_presentation_live(&self) -> bool {
        let now = self.compositor_now();
        self.chart_live.values().any(|until| *until > now)
    }

    pub fn glyph_presentation_live(&self) -> bool {
        let now = self.compositor_now();
        self.glyph_live
            .values()
            .any(|until| until.is_none_or(|until| until > now))
    }

    /// Drop the node's primitives that `doomed` names, and their places in
    /// `ordered`.
    ///
    /// The key comes off the primitive rather than out of a fresh ancestor
    /// walk: a key computed against a node the delta has already replaced
    /// would not match the one this primitive entered `ordered` at, and would
    /// leave it there forever.
    fn retire_node_primitives(
        &mut self,
        id: StableNodeId,
        doomed: impl Fn(&RetainedPrimitive) -> bool,
    ) {
        let slots = self
            .node_primitives(id)
            .filter(|(_, held)| doomed(held))
            .map(|(slot, _)| *slot)
            .collect::<Vec<_>>();
        for slot in slots {
            if let Some(held) = self.primitives.remove(&slot) {
                self.ordered.remove(&held.key);
                self.structure_changed = true;
            }
        }
    }

    #[cfg(test)]
    fn primitives_for_node(&self, node: StableNodeId) -> impl Iterator<Item = &ScenePrimitive> {
        self.node_primitives(node).map(|(_, held)| &held.primitive)
    }

    fn node_primitive_count(&self, node: StableNodeId) -> usize {
        self.node_primitives(node).count()
    }

    fn node_primitives(
        &self,
        node: StableNodeId,
    ) -> impl Iterator<Item = (&PrimitiveId, &RetainedPrimitive)> {
        self.primitives.range(
            PrimitiveId { node, slot: 0 }..=PrimitiveId {
                node,
                slot: u64::MAX,
            },
        )
    }

    /// The paint-order key.
    ///
    /// Every primitive of a node sits under the same stack — the z-index and
    /// document order in it are the node's — so while the rebuild pass runs it
    /// is computed once per node and every primitive shares that `Arc`.
    fn scene_order_key(&self, primitive: &ScenePrimitive) -> SceneOrderKey {
        if let Some(key) = triggered_overlay_surface_key(primitive) {
            return key;
        }
        if let Ok(mut scratch) = self.rebuild_scratch.lock()
            && scratch.active
        {
            if let Some((node, stack)) = scratch.order_stack.as_ref()
                && *node == primitive.node
            {
                return SceneOrderKey::at(Arc::clone(stack), primitive);
            }
            let prefix = self.group_prefix_of(&mut scratch, primitive.node);
            let stack = order_stack(&self.nodes, &prefix, primitive);
            // A painted node's primitives do not share one stack.
            if self.custom_paint.is_empty() || !has_custom_paint(&self.nodes, primitive.node) {
                scratch.order_stack = Some((primitive.node, Arc::clone(&stack)));
            }
            return SceneOrderKey::at(stack, primitive);
        }
        order_key(&self.nodes, &self.node_order, primitive)
    }

    /// A node's paint-order prefix: what it inherits from the chain above it,
    /// plus its own entry when it opens a stacking group.
    ///
    /// The inherited half is what siblings share, so it is the half worth
    /// remembering. A node that opens no group of its own **shares** that
    /// `Arc` rather than copying it.
    fn group_prefix_of(&self, scratch: &mut RebuildScratch, node: StableNodeId) -> GroupPrefix {
        let Some(candidate) = self.nodes.get(&node) else {
            return Arc::from(Vec::new());
        };
        // A viewport-fixed node starts its own chain: a triggered menu must not
        // stay inside the isolation group of whatever opened it.
        let inherited =
            if candidate.source_style.layout.position == nana_ui_core::PositionSpec::Fixed {
                Arc::from(Vec::new())
            } else {
                match candidate.parent {
                    Some(parent) => {
                        if let Some((held, prefix)) = scratch.inherited_prefix.as_ref()
                            && *held == parent
                        {
                            Arc::clone(prefix)
                        } else {
                            let prefix: Arc<[(i32, usize)]> =
                                group_prefix(&self.nodes, &self.node_order, parent).into();
                            scratch.inherited_prefix = Some((parent, Arc::clone(&prefix)));
                            prefix
                        }
                    }
                    None => Arc::from(Vec::new()),
                }
            };
        if !is_stacking_group(&self.nodes, candidate) {
            return inherited;
        }
        let mut prefix = inherited.to_vec();
        prefix.push((
            candidate.z_index,
            self.node_order.get(&node).copied().unwrap_or(0),
        ));
        Arc::from(prefix)
    }

    fn node_descends_from(&self, id: StableNodeId, ancestor: StableNodeId) -> bool {
        let mut current = Some(id);
        while let Some(candidate) = current {
            if candidate == ancestor {
                return true;
            }
            current = self.nodes.get(&candidate).and_then(|node| node.parent);
        }
        false
    }

    fn insert_primitive(&mut self, primitive: ScenePrimitive) {
        let key = self.scene_order_key(&primitive);
        let build = self.build;
        if let Some(held) = self.primitives.get_mut(&primitive.id) {
            // A rebuild usually puts the same primitive back in the same
            // place. Re-entering `ordered` at a key it already holds is the
            // work this avoids.
            let moved = held.key != key;
            // A frame plan names custom nodes by renderer and resource, so one
            // slot swapping which it draws is a structural change even though
            // the slot itself stays.
            self.structure_changed |=
                Self::custom_binding(&held.primitive.kind) != Self::custom_binding(&primitive.kind);
            // A painter moving `draw_default()` to the other side of the
            // children re-keys the same slots with nothing else changing: the
            // plan has to hear it (Issue #217).
            self.structure_changed |= moved
                && !self.custom_paint.is_empty()
                && has_custom_paint(&self.nodes, primitive.node);
            let previous = std::mem::replace(&mut held.key, key.clone());
            held.primitive = primitive;
            held.build = build;
            if moved {
                self.ordered.remove(&previous);
                self.ordered.insert(key);
            }
            return;
        }
        self.structure_changed = true;
        self.primitives.insert(
            primitive.id,
            RetainedPrimitive {
                primitive,
                key: key.clone(),
                build,
            },
        );
        self.ordered.insert(key);
    }
}

fn collect_unextracted_descendants(
    nodes: &SceneNodes,
    root: StableNodeId,
    extracted: &NodeSet,
    out: &mut Vec<StableNodeId>,
) {
    let Some(node) = nodes.get(&root) else {
        return;
    };
    for &child in node.children.iter() {
        if !extracted.contains(&child) {
            out.push(child);
        }
        collect_unextracted_descendants(nodes, child, extracted, out);
    }
}

#[derive(PartialEq)]
enum InheritedComponentClip {
    EmptyState(LayoutBox),
    ModalFrame {
        surface: LayoutBox,
        body: LayoutBox,
        body_root: Option<StableNodeId>,
    },
}

fn inherited_component_clip(node: &ExtractedNode) -> Option<InheritedComponentClip> {
    match node.component_geometry.as_deref() {
        Some(ComponentGeometry::EmptyState { root_clip, .. }) => {
            Some(InheritedComponentClip::EmptyState(*root_clip))
        }
        Some(ComponentGeometry::ModalFrame { surface, body, .. }) => {
            let body_root = match node.standard_visual.as_ref() {
                Some(StandardVisual::ModalFrame { slots, .. }) => slots.body,
                _ => None,
            };
            Some(InheritedComponentClip::ModalFrame {
                surface: *surface,
                body: *body,
                body_root,
            })
        }
        _ => None,
    }
}

fn local_opacity(node: &ExtractedNode) -> f32 {
    node.source_style
        .layout
        .opacity
        .unwrap_or(1.0)
        .clamp(0.0, 1.0)
}

/// Whether re-extracting this node can move a *retained descendant's* projected
/// transform, which is the only way one node's change reaches another node's
/// visibility bound.
///
/// `project_ancestor_state` builds that transform from each ancestor's
/// `position` (where the chain starts), its local scene transform, and whether
/// it closes a 3D context — and nothing else. Everything else a style carries
/// still bumps `attribute_epoch`, because the *clip* chain paint reads is wider
/// than this; it just does not move a bound, so it must not cost a pass over
/// every primitive in the scene. A fade is the case that matters: it rewrites
/// one node's style every frame and moves nothing.
fn inherited_transform_changed(old: &ExtractedNode, node: &ExtractedNode) -> bool {
    if old.parent != node.parent
        || old.layout != node.layout
        || old.source_style.layout.position != node.source_style.layout.position
        || old.source_style.layout.fails_closed_3d_context()
            != node.source_style.layout.fails_closed_3d_context()
    {
        return true;
    }
    [false, true].into_iter().any(|blocks_3d| {
        node_scene_transform(old.source_style.layout.as_ref(), old.layout, blocks_3d)
            != node_scene_transform(node.source_style.layout.as_ref(), node.layout, blocks_3d)
    })
}

fn is_workspace_resize_handle(node: &ExtractedNode) -> bool {
    matches!(
        node.kind.as_ref(),
        NodeKind::Element { tag } if tag == "workspace-resize-handle"
    )
}

fn is_descendant_of_rasterized_svg(nodes: &SceneNodes, node: &ExtractedNode) -> bool {
    let Some(parent) = node.parent else {
        return false;
    };
    for (_, parent) in ancestor_nodes(nodes, parent) {
        if parent
            .custom_render
            .as_ref()
            .is_some_and(|custom| custom.renderer.as_ref() == "nana.host-texture")
            && matches!(
                parent.kind.as_ref(),
                NodeKind::Element { tag } if tag.eq_ignore_ascii_case("svg")
            )
        {
            return true;
        }
    }
    false
}

/// Whether `node` is part of a glyph an ancestor already paints.
///
/// An icon bound to `IconGlyph` keeps the vector markup it came from — `path`,
/// `circle` — as children. The atlas glyph draws all of it, so those children
/// must not paint their own boxes on top of it.
///
/// They are *in flow* inside the icon, and that is the part worth naming. A
/// surface an icon only hosts is not: an `IconButton` parents its tooltip to
/// itself, fixed-positioned at `z_index` 1000, and a walk that stopped at the
/// first `Icon` ancestor swallowed it — which is why no icon-button tooltip has
/// ever reached the scene. The walk therefore ends at the first out-of-flow
/// box, and the icon's own children keep being skipped.
fn is_descendant_of_icon_visual(nodes: &SceneNodes, node: &ExtractedNode) -> bool {
    if node.source_style.layout.is_out_of_flow() {
        return false;
    }
    let Some(parent) = node.parent else {
        return false;
    };
    for (_, ancestor) in ancestor_nodes(nodes, parent) {
        if matches!(ancestor.standard_visual, Some(StandardVisual::Icon { .. })) {
            return true;
        }
        if ancestor.source_style.layout.is_out_of_flow() {
            return false;
        }
    }
    false
}

fn has_extracted_child(nodes: &SceneNodes, node: &ExtractedNode) -> bool {
    node.children.iter().any(|child| nodes.contains_key(child))
}

fn dest_filter_applies(nodes: &SceneNodes, node: &ExtractedNode) -> bool {
    let Some(filter) = node
        .source_style
        .layout
        .paint
        .filter
        .filter(|filter| !filter.is_identity())
    else {
        return false;
    };
    filter.blur_radius > 0.0
        || filter.drop_shadow.is_some()
        || has_extracted_child(nodes, node)
        || node
            .text
            .as_ref()
            .is_some_and(|text| !text.value.is_empty())
        || node.custom_render.is_some()
}

/// What a node's own paint style contributes to the paint-order keys of its
/// subtree — the reason a change to it costs a reorder.
///
/// The *value* of `opacity` is not in it: [`is_opacity_group`] only asks
/// whether the node is translucent at all, so fading a container from 0.35 to
/// 0.37 must not re-sort the scene. Neither is an identity filter, which
/// [`dest_filter_applies`] already reads as no filter.
#[derive(PartialEq)]
struct PaintOrderFacts {
    z_index: i32,
    translucent: bool,
    filter: Option<ColorFilter>,
    mix_blend: MixBlendMode,
    stacking_context: bool,
    custom_paint: bool,
}

fn paint_order_facts(node: &ExtractedNode) -> PaintOrderFacts {
    let paint = &node.source_style.layout.paint;
    PaintOrderFacts {
        z_index: node.z_index,
        translucent: is_translucent(node),
        filter: paint.filter.filter(|filter| !filter.is_identity()),
        mix_blend: paint.mix_blend,
        stacking_context: node.source_style.layout.creates_paint_stacking_context(),
        custom_paint: node.custom_paint.is_some(),
    }
}

/// Scene size up to which `apply_delta` re-derives what it retained under
/// `debug_assertions` — [`UiScene::dest_group_candidates`] and, in
/// [`UiScene::audit_retained_projection`], the paint order, the frame plan and
/// the visibility index. Unit-test scenes are a handful of nodes; product
/// scenes are thousands, and auditing those on every delta would slow debug
/// builds without testing anything the small scenes do not.
const RETAINED_AUDIT_LIMIT: usize = 512;

/// The only thing [`is_opacity_group`] — and so every descendant's paint-order
/// key — asks of a node's opacity. Keep the readings together: a group test
/// that started caring about the value itself would need `stacking_changed` and
/// [`may_be_dest_group`] to care again too.
fn is_translucent(node: &ExtractedNode) -> bool {
    let opacity = local_opacity(node);
    opacity > 0.0 && opacity < 1.0
}

/// Node-local necessary condition for [`is_opacity_group`].
///
/// Every disjunct there needs one of these to hold, and none of them can be
/// turned on by a *different* node, so a scene where no node passes this has no
/// opacity group whatever its shape. [`UiScene::dest_group_candidates`] counts
/// these as nodes are inserted and removed, which is why this must stay a
/// superset: a term added to [`is_opacity_group`] needs its own term here.
fn may_be_dest_group(node: &ExtractedNode) -> bool {
    is_translucent(node)
        || node
            .source_style
            .layout
            .paint
            .filter
            .is_some_and(|filter| !filter.is_identity())
        || !node.source_style.layout.paint.mix_blend.is_normal()
}

fn is_opacity_group(nodes: &SceneNodes, node: &ExtractedNode) -> bool {
    let translucent = is_translucent(node) && has_extracted_child(nodes, node);
    translucent
        || dest_filter_applies(nodes, node)
        || !node.source_style.layout.paint.mix_blend.is_normal()
}

/// Whether the two styles differ in anything a descendant inherits the
/// *projection* of — the transform it sits under, the clips it is cut by,
/// where the fixed boundary is.
///
/// `opacity` is left out. It does reach descendants, but not through the
/// projection: [`inherited_opacity`] decides whether it moved and asks for the
/// subtree to be rebuilt, and a fade that keeps a container a group does not
/// move anything at all. Bumping the attribute epoch for it would make every
/// descendant re-derive its transform and clips on the next frame's draw.
///
/// Every other field is compared as a whole, so a field added later cannot be
/// forgotten here.
fn inherited_layout_changed(
    old: &Arc<nana_ui_core::LayoutStyle>,
    new: &Arc<nana_ui_core::LayoutStyle>,
) -> bool {
    if Arc::ptr_eq(old, new) || old == new {
        return false;
    }
    if old.opacity == new.opacity {
        return true;
    }
    let mut without_the_fade = old.as_ref().clone();
    without_the_fade.opacity = new.opacity;
    &without_the_fade != new.as_ref()
}

/// The opacity this node multiplies into every descendant's primitive.
///
/// A group composites its subtree as one layer, so its own opacity is applied
/// there once and never reaches a descendant's primitive.
fn inherited_opacity(nodes: &SceneNodes, node: &ExtractedNode) -> f32 {
    if is_opacity_group(nodes, node) {
        1.0
    } else {
        local_opacity(node)
    }
}

fn is_stacking_group(nodes: &SceneNodes, node: &ExtractedNode) -> bool {
    // A painted node keeps its children between its two paint phases, so it
    // has to own their order the way a stacking context does.
    node.custom_paint.is_some()
        || is_opacity_group(nodes, node)
        || (has_extracted_child(nodes, node)
            && node.source_style.layout.creates_paint_stacking_context())
}

fn is_filter_group(nodes: &SceneNodes, node: &ExtractedNode) -> bool {
    dest_filter_applies(nodes, node)
}

fn is_dest_group(nodes: &SceneNodes, node: &ExtractedNode) -> bool {
    is_opacity_group(nodes, node)
}

fn inset_shadow_overlay(node: &ExtractedNode) -> Option<InsetShadowOverlay> {
    let shadow = node
        .source_style
        .layout
        .paint
        .box_shadows
        .iter()
        .copied()
        .find(|shadow| shadow.inset)?;
    Some(InsetShadowOverlay {
        elevation: ComponentElevation::from_box_shadow(shadow),
        bounds: SceneRect {
            x: node.layout.x,
            y: node.layout.y,
            width: node.layout.width,
            height: node.layout.height,
        },
        corner_radius: surface_corner_radii(
            node.source_style.layout.as_ref(),
            node.layout.width,
            node.layout.height,
        ),
    })
}

/// A parent chain longer than this is a bug in extraction, not a deep tree.
///
/// It replaces the per-call `HashSet` these walks used to carry as a cycle
/// guard. They run once per primitive per frame — a set allocation each cost
/// more than the walk it was protecting, and a depth cap protects the same
/// thing.
pub(super) const MAX_ANCESTOR_DEPTH: usize = 4096;

/// `node` and its ancestors by id, innermost first.
///
/// Yields an id whether or not the scene still holds that node, and stops
/// after one it does not: the chain cannot continue past a node whose parent
/// nobody knows.
pub(super) fn ancestor_ids(
    nodes: &SceneNodes,
    node: StableNodeId,
) -> impl Iterator<Item = StableNodeId> + '_ {
    let mut current = Some(node);
    let mut depth = 0usize;
    std::iter::from_fn(move || {
        let id = current.take()?;
        if depth >= MAX_ANCESTOR_DEPTH {
            return None;
        }
        depth += 1;
        current = nodes.get(&id).and_then(|node| node.parent);
        Some(id)
    })
}

/// `node` and its ancestors, innermost first, stopping at the first one the
/// scene no longer holds.
fn ancestor_nodes(
    nodes: &SceneNodes,
    node: StableNodeId,
) -> impl Iterator<Item = (StableNodeId, &ExtractedNode)> {
    let mut current = Some(node);
    let mut depth = 0usize;
    std::iter::from_fn(move || {
        let id = current.take()?;
        if depth >= MAX_ANCESTOR_DEPTH {
            return None;
        }
        depth += 1;
        let entry = nodes.get(&id)?;
        current = entry.parent;
        Some((id, entry.as_ref()))
    })
}

/// Dest groups per node, stamped with the scene instance they were read at.
type OpacityGroupCache = NodeMap<(u64, Arc<[OpacityGroup]>)>;

/// Ancestor layer factors, stamped with the scene instance and the attribute
/// epoch.
pub(super) type LayerFactorCache = NodeMap<((u64, u64), f32)>;

/// The scene's nodes.
///
/// `Arc` rather than the node itself: rebuilding a node's primitives needs an
/// owned handle while `&mut self` inserts them, and an `ExtractedNode` is 784
/// bytes. A container style change rebuilds every descendant, so that clone
/// used to be most of a megabyte of memmove per frame.
pub(super) type SceneNodes = NodeMap<Arc<ExtractedNode>>;

/// Which chain an [`AncestorState`] was read for: the parent it starts at,
/// whether the node breaks out of it, and whether the caller wanted the
/// presented values or the logical ones.
type AncestorStateKey = (StableNodeId, bool, bool);

/// What a remembered draw-time answer is stamped with. See
/// [`UiScene::draw_stamp`].
type DrawStamp = (u64, u64);

/// What a node inherits from the chain above it.
type AncestorState = (AffineTransform, f32, Arc<[ClipRegion]>, bool);

/// One `(z-index, document order)` per stacking group above a node, outermost
/// first.
type GroupPrefix = Arc<[(i32, usize)]>;

/// Two answers the primitive-rebuild pass keeps asking for: what a node
/// inherits from the chain above it, and where that chain puts it in paint
/// order. A container style change rebuilds every descendant, and a node owns
/// several primitives, so both are asked many times for the same parent.
///
/// One entry each, not maps: the pass walks the subtree in order, so the next
/// question almost always has the same answer as the last, and comparing two
/// words beats hashing. A miss costs exactly what this used to cost every time.
///
/// Deliberately not stamped and not always on. It is filled only while
/// `apply_delta` rebuilds primitives — where `self.nodes` and `self.node_order`
/// are already final and cannot move under it — and emptied on the way out.
#[derive(Default, Debug)]
struct RebuildScratch {
    active: bool,
    ancestor_state: Option<(AncestorStateKey, AncestorState)>,
    /// The same answer for the draw pass, which has no bracket to reset it:
    /// siblings ask for it one after another while the painter walks paint
    /// order, so one entry carries a whole container. Stamped, because nothing
    /// clears it. See [`UiScene::draw_stamp`].
    drawn_ancestor_state: Option<(AncestorStateKey, DrawStamp, AncestorState)>,
    /// The paint-order stack of the node being rebuilt, shared by all of its
    /// primitives.
    order_stack: Option<(StableNodeId, GroupPrefix)>,
    /// The same, for the chain *above* a node: siblings share it, so the pass
    /// pays the walk once per container instead of once per node.
    inherited_prefix: Option<(StableNodeId, GroupPrefix)>,
}

impl RebuildScratch {
    fn begin(&mut self) {
        self.active = true;
        self.ancestor_state = None;
        self.order_stack = None;
        self.inherited_prefix = None;
    }

    fn end(&mut self) {
        self.active = false;
        self.ancestor_state = None;
        self.order_stack = None;
        self.inherited_prefix = None;
    }
}

/// The list every node that is not itself a group shares with its parent.
fn empty_opacity_groups() -> Arc<[OpacityGroup]> {
    static EMPTY: std::sync::OnceLock<Arc<[OpacityGroup]>> = std::sync::OnceLock::new();
    Arc::clone(EMPTY.get_or_init(|| Arc::from(Vec::new())))
}

fn dest_group(nodes: &SceneNodes, id: StableNodeId, candidate: &ExtractedNode) -> OpacityGroup {
    OpacityGroup {
        node: id,
        opacity: local_opacity(candidate),
        filter: if dest_filter_applies(nodes, candidate) {
            candidate
                .source_style
                .layout
                .paint
                .filter
                .unwrap_or_default()
        } else {
            ColorFilter::default()
        },
        mix_blend: candidate.source_style.layout.paint.mix_blend,
        inset_shadow: inset_shadow_overlay(candidate),
    }
}

/// The clip an `overflow` other than `visible` puts on a node's descendants.
///
/// It is the border box with the box's own rounding, as in CSS: a child
/// cannot paint past the rounded corners of an `overflow: hidden` parent.
/// The scene carries one radius per clip, so corners of unequal radius clip
/// with the smallest — never cutting away paint a corner should keep. With
/// only one axis clipped the open axis extends far past the box and there is
/// no corner to round.
fn overflow_clip_region(
    style: &nana_ui_core::LayoutStyle,
    bounds: SceneRect,
    transform: AffineTransform,
) -> Option<ClipRegion> {
    let (x, y, width, height) =
        style.overflow_clip_box(bounds.x, bounds.y, bounds.width, bounds.height)?;
    let corner_radius = if style.overflow_x.clips() && style.overflow_y.clips() {
        style
            .resolved_border_radii(bounds.width, bounds.height)
            .into_iter()
            .fold(f32::INFINITY, f32::min)
            .min(bounds.width.min(bounds.height) * 0.5)
    } else {
        0.0
    };
    Some(ClipRegion {
        bounds: SceneRect {
            x,
            y,
            width,
            height,
        },
        transform,
        corner_radius: if corner_radius.is_finite() {
            corner_radius.max(0.0)
        } else {
            0.0
        },
        polygon_clip: None,
    })
}

fn clip_path_region(
    style: &nana_ui_core::LayoutStyle,
    bounds: SceneRect,
    transform: AffineTransform,
) -> Option<ClipRegion> {
    let clip_path = style.paint.clip_path.as_ref()?;
    match clip_path {
        ClipPath::Inset(inset) => {
            let [top, right, bottom, left] = inset.resolve_offsets(bounds.width, bounds.height);
            Some(ClipRegion {
                bounds: SceneRect {
                    x: bounds.x + left,
                    y: bounds.y + top,
                    width: (bounds.width - left - right).max(0.0),
                    height: (bounds.height - top - bottom).max(0.0),
                },
                transform,
                corner_radius: inset.resolve_round(bounds.width, bounds.height),
                polygon_clip: None,
            })
        }
        ClipPath::Polygon(_) => {
            let points = clip_path.resolve_polygon_points(bounds.width, bounds.height)?;
            let min_x = points
                .iter()
                .map(|point| point[0])
                .fold(f32::INFINITY, f32::min);
            let min_y = points
                .iter()
                .map(|point| point[1])
                .fold(f32::INFINITY, f32::min);
            let max_x = points
                .iter()
                .map(|point| point[0])
                .fold(f32::NEG_INFINITY, f32::max);
            let max_y = points
                .iter()
                .map(|point| point[1])
                .fold(f32::NEG_INFINITY, f32::max);
            let local_points = points
                .iter()
                .map(|point| [point[0] - min_x, point[1] - min_y])
                .collect();
            Some(ClipRegion {
                bounds: SceneRect {
                    x: bounds.x + min_x,
                    y: bounds.y + min_y,
                    width: (max_x - min_x).max(0.0),
                    height: (max_y - min_y).max(0.0),
                },
                transform,
                corner_radius: 0.0,
                polygon_clip: Some(local_points),
            })
        }
        ClipPath::Circle(_) | ClipPath::Ellipse(_) => {
            let [x, y, w, h] = clip_path.resolve_ellipse_rect(bounds.width, bounds.height)?;
            Some(ClipRegion::ellipse(
                SceneRect {
                    x: bounds.x + x,
                    y: bounds.y + y,
                    width: w.max(0.0),
                    height: h.max(0.0),
                },
                transform,
            ))
        }
    }
}

fn filter_groups_from(nodes: &SceneNodes, node: StableNodeId) -> Vec<FilterGroup> {
    let mut groups = Vec::new();
    for (id, candidate) in ancestor_nodes(nodes, node) {
        if is_filter_group(nodes, candidate) {
            groups.push(FilterGroup {
                node: id,
                filter: candidate
                    .source_style
                    .layout
                    .paint
                    .filter
                    .unwrap_or_default(),
            });
        }
    }
    groups.reverse();
    groups
}

/// `(z_index, document_order)` of every isolating stacking group above (and
/// including) `node`, outermost first. Opacity / filter / mix-blend dest groups
/// plus `isolation` and positioned + `z-index`.
fn group_prefix(
    nodes: &SceneNodes,
    node_order: &NodeMap<usize>,
    node: StableNodeId,
) -> Vec<(i32, usize)> {
    let mut stack = Vec::new();
    for (id, candidate) in ancestor_nodes(nodes, node) {
        if is_stacking_group(nodes, candidate) {
            let z_index = candidate.z_index;
            let order = node_order.get(&id).copied().unwrap_or(0);
            stack.push((z_index, order));
        }
        if candidate.source_style.layout.position == nana_ui_core::PositionSpec::Fixed {
            // Triggered menus and overlay surfaces are viewport-fixed. Their
            // paint order must not stay inside a parent's isolation group, or
            // a later sibling card would cover an open Popover.
            break;
        }
    }
    stack.reverse();
    stack
}

fn has_custom_paint(nodes: &SceneNodes, node: StableNodeId) -> bool {
    nodes
        .get(&node)
        .is_some_and(|node| node.custom_paint.is_some())
}

fn primitive_paint_layer(slot: u64) -> u64 {
    match slot >> 32 {
        // Behind the glyphs (layer 2), above a custom-rendered backdrop
        // (layer 1, broken by the much larger slot).
        namespace if namespace == u64::from(DOCUMENT_TEXT_SELECTION) => 1,
        namespace if namespace == u64::from(TEXT_LINE_LABELS) => 40,
        namespace if namespace == u64::from(TEXT_DIAGNOSTIC_MARKERS) => 20,
        namespace if namespace == u64::from(TEXT_DIAGNOSTIC_LABELS) => 58,
        namespace if namespace == u64::from(TEXT_ATOM_ICONS) => 27,
        namespace if namespace == u64::from(TEXT_ATOM_LABELS) => 32,
        _ => slot,
    }
}

fn order_key(
    nodes: &SceneNodes,
    node_order: &NodeMap<usize>,
    primitive: &ScenePrimitive,
) -> SceneOrderKey {
    if let Some(key) = triggered_overlay_surface_key(primitive) {
        return key;
    }
    let prefix: GroupPrefix = group_prefix(nodes, node_order, primitive.node).into();
    SceneOrderKey::at(order_stack(nodes, &prefix, primitive), primitive)
}

/// The stacking entries a node's primitives paint under, outermost first.
pub(super) fn order_stack(
    nodes: &SceneNodes,
    prefix: &GroupPrefix,
    primitive: &ScenePrimitive,
) -> GroupPrefix {
    // A group's own paint is the prefix, before its descendants. Repeating its
    // outer z-index here incorrectly puts lower-z children behind its backplate.
    if let Some(node) = nodes.get(&primitive.node)
        && is_stacking_group(nodes, node)
    {
        // A painted node splits its own paint around its children.
        if let Some(entry) = node
            .custom_paint
            .as_deref()
            .and_then(|recording| custom_paint::custom_paint_stack(recording, primitive.id.slot))
        {
            let mut stack = Vec::with_capacity(prefix.len() + 1);
            stack.extend_from_slice(prefix);
            stack.push(entry);
            return Arc::from(stack);
        }
        return Arc::clone(prefix);
    }
    let mut stack = Vec::with_capacity(prefix.len() + 1);
    stack.extend_from_slice(prefix);
    stack.push((primitive.z_index, primitive.document_order));
    Arc::from(stack)
}

impl SceneOrderKey {
    fn at(stack: GroupPrefix, primitive: &ScenePrimitive) -> Self {
        Self {
            stack,
            paint_layer: primitive_paint_layer(primitive.id.slot),
            slot: primitive.id.slot,
            node: primitive.node,
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn scene_rect(bounds: LayoutBox) -> SceneRect {
    SceneRect {
        x: bounds.x,
        y: bounds.y,
        width: bounds.width.max(0.0),
        height: bounds.height.max(0.0),
    }
}

/// Draws what moves with an editor's text along x under its scroll's
/// translation, each box less it: the painter snaps that translation, so the
/// value keeps its glyphs and the caret, selection and marks stay on them.
#[derive(Clone, Copy)]
struct EditorScroll(Option<TextInputScroll>);

impl EditorScroll {
    fn transform(self, transform: AffineTransform) -> AffineTransform {
        match self.0 {
            Some(scroll) => transform.then(AffineTransform::from_matrix([
                1.0,
                0.0,
                0.0,
                1.0,
                scroll.offset_x,
                0.0,
            ])),
            None => transform,
        }
    }

    /// A box that moves with the text, in the translated space.
    fn rect(self, bounds: LayoutBox) -> SceneRect {
        let mut rect = scene_rect(bounds);
        if let Some(scroll) = self.0 {
            rect.x -= scroll.offset_x;
        }
        rect
    }

    /// Text over the value: a diagnostic's message, an atom's label.
    fn text(self, mut primitive: ScenePrimitive) -> ScenePrimitive {
        if let Some(scroll) = self.0 {
            primitive.bounds.x -= scroll.offset_x;
            primitive.transform = self.transform(primitive.transform);
        }
        primitive
    }

    /// The value, at its exact unscrolled x.
    fn value(self, primitive: ScenePrimitive) -> ScenePrimitive {
        let mut primitive = self.text(primitive);
        if let Some(scroll) = self.0 {
            primitive.bounds.x = scroll.text_x;
        }
        primitive
    }
}

#[allow(clippy::too_many_arguments)]
fn paint_select_handle(
    emit: &mut impl FnMut(ScenePrimitive),
    id: StableNodeId,
    handle: &LayoutBox,
    color: [f32; 4],
    transform: AffineTransform,
    clips: &Arc<[ClipRegion]>,
    opacity: f32,
    z_index: i32,
    document_order: usize,
) {
    let center_x = handle.x + handle.width / 2.0;
    let center_y = handle.y + handle.height / 2.0;
    let widths = [8.0, 6.0, 4.0, 2.0];
    for (index, width) in widths.iter().copied().enumerate() {
        emit(visual_quad(
            &VisualPrimitiveContext {
                node: id,
                transform,
                clips,
                opacity,
                z_index,
                document_order,
            },
            3 + index as u64,
            SceneRect {
                x: center_x - width / 2.0,
                y: center_y - 1.5 + index as f32,
                width,
                height: 1.0,
            },
            VisualQuadStyle::solid(color),
        ));
    }
}

fn component_geometry_owns_text(geometry: Option<&ComponentGeometry>) -> bool {
    matches!(
        geometry,
        Some(
            ComponentGeometry::Button { .. }
                | ComponentGeometry::TextInput { .. }
                | ComponentGeometry::Switch { .. }
                | ComponentGeometry::Range { .. }
                | ComponentGeometry::Card { .. }
                | ComponentGeometry::StatusBadge { .. }
                | ComponentGeometry::ValidationMessage { .. }
                | ComponentGeometry::EmptyState { .. }
                | ComponentGeometry::LabeledValue { .. }
                | ComponentGeometry::SelectionOption { .. }
                | ComponentGeometry::ModalFrame { .. }
                | ComponentGeometry::Progress { .. }
                | ComponentGeometry::FormField { .. }
                | ComponentGeometry::Select { .. }
                | ComponentGeometry::ActionMenuItem { .. }
                | ComponentGeometry::MenuSurface { .. }
                | ComponentGeometry::TreeView { .. }
                | ComponentGeometry::CommandPalette { .. }
                | ComponentGeometry::CalendarHeatmap { .. }
                | ComponentGeometry::ReorderList { .. }
                | ComponentGeometry::NativeMarkdown { .. }
                | ComponentGeometry::SelectableRichText { .. }
                | ComponentGeometry::GraphCanvas { .. }
                | ComponentGeometry::ImageViewer { .. }
                | ComponentGeometry::KeyCaptureLayer { .. }
                | ComponentGeometry::KeymapLayer { .. }
        )
    )
}

#[allow(clippy::too_many_arguments)]
fn component_text_primitive(
    id: StableNodeId,
    slot: u64,
    region: &ComponentTextRegion,
    horizontal_alignment: TextHorizontalAlignment,
    ellipsis: bool,
    node: &ExtractedNode,
    transform: AffineTransform,
    clips: Arc<[ClipRegion]>,
    opacity: f32,
    document_order: usize,
) -> ScenePrimitive {
    let paint_color = node
        .style
        .paint_colors
        .color
        .filter(|paint| region.color.is_none_or(|color| color == paint.to_srgb()));
    let multiline = matches!(
        node.component_geometry.as_deref(),
        Some(ComponentGeometry::TextInput {
            multiline: true,
            ..
        })
    );
    let intrinsic_multiline = matches!(
        node.standard_visual,
        Some(StandardVisual::EmptyState { .. } | StandardVisual::ModalFrame { .. })
    ) || matches!(
        node.component_geometry.as_deref(),
        Some(
            ComponentGeometry::NativeMarkdown { .. } | ComponentGeometry::SelectableRichText { .. }
        )
    );
    ScenePrimitive {
        id: PrimitiveId { node: id, slot },
        node: id,
        bounds: scene_rect(region.bounds),
        transform,
        clips,
        opacity,
        z_index: node.z_index,
        document_order,
        kind: ScenePrimitiveKind::Text {
            content: region.content.clone(),
            color: slot_color(
                paint_color,
                region
                    .color
                    .or_else(|| {
                        node.style
                            .paint_colors
                            .color
                            .map(nana_ui_core::PaintColor::to_srgb)
                    })
                    .or(node.style.color),
            ),
            size: region.font_size,
            weight: region.font_weight,
            family: node.style.font_family.as_deref().map(str::to_owned),
            line_height: node.style.line_height,
            letter_spacing: node.style.letter_spacing,
            wrap: multiline || intrinsic_multiline,
            ellipsis,
            max_lines: None,
            shaping: if node.editable {
                TextShaping::Advanced
            } else {
                TextShaping::Auto
            },
            horizontal_alignment,
            vertical_alignment: if multiline || intrinsic_multiline {
                TextVerticalAlignment::Top
            } else {
                TextVerticalAlignment::Center
            },
            spans: scene_text_spans(node, Some(region), region.content.as_ref()),
            text_shadow: node.source_style.layout.paint.text_shadow,
            underline: node
                .source_style
                .layout
                .text_decoration
                .is_some_and(|d| d.underline),
            line_through: node
                .source_style
                .layout
                .text_decoration
                .is_some_and(|d| d.line_through),
            font_features: node
                .source_style
                .layout
                .font_features
                .clone()
                .unwrap_or_default(),
            italic: node.style.italic,
            wrap_break: node.source_style.layout.text_wrap_break(),
            opentype: SceneTextOpenType {
                // What this node was measured with. An editor's value keeps
                // the newlines the user typed whatever `white-space` says, and
                // a renderer that folds them paints one line where six were
                // measured.
                preserve_lines: node.text_preserve_lines,
                ..SceneTextOpenType::from_computed(&node.style)
            },
            layout: None,
            rich: None,
            presentation: None,
        },
    }
}

fn surface_corner_radii(style: &nana_ui_core::LayoutStyle, width: f32, height: f32) -> [f32; 4] {
    let mut radii = style.resolved_border_radii(width, height);
    if let Some(ClipPath::Inset(inset)) = style.paint.clip_path.as_ref() {
        let round = inset.resolve_round(width, height);
        if round > 0.0 {
            radii = radii.map(|radius| radius.max(round));
        }
    }
    radii
}

fn scene_text_spans(
    node: &ExtractedNode,
    region: Option<&ComponentTextRegion>,
    content: &str,
) -> Vec<SceneTextSpan> {
    if node.text_spans.is_empty() {
        return Vec::new();
    }
    // TextInput 主文本区域：span 由 extraction 按 TextInput 呈现内容
    // （折叠态经显示视图重投后的显示串）空间产出，直接按 content 校验
    // 边界。其余区域（行号标签、钉住行、Select 标题、普通文本节点等）
    // 不承载该空间，维持值空间等值守卫——内容与 span 空间不一致时整批
    // 丢弃。
    let editor_main_text = region.is_some_and(|region| {
        matches!(
            node.component_geometry.as_deref(),
            Some(ComponentGeometry::TextInput { text, .. }) if std::ptr::eq(text, region)
        )
    });
    if !editor_main_text {
        let Some(source) = node.text.as_ref() else {
            return Vec::new();
        };
        if source.value != content {
            return Vec::new();
        }
    }
    node.text_spans
        .iter()
        .filter(|span| {
            span.start < span.end
                && span.end <= content.len()
                && content.is_char_boundary(span.start)
                && content.is_char_boundary(span.end)
        })
        .map(|span| SceneTextSpan {
            start: span.start,
            end: span.end,
            color: slot_color(span.paint_color, Some(span.color))
                .expect("a span has a resolved colour"),
        })
        .collect()
}

struct VisualPrimitiveContext<'a> {
    node: StableNodeId,
    transform: AffineTransform,
    clips: &'a Arc<[ClipRegion]>,
    opacity: f32,
    z_index: i32,
    document_order: usize,
}

struct VisualQuadStyle {
    background: Option<[f32; 4]>,
    border_color: Option<[f32; 4]>,
    border_width: f32,
    corner_radius: [f32; 4],
}

impl VisualQuadStyle {
    /// 纯色填充：无边框、直角。装饰条、参考线、高亮底色共用。
    fn solid(background: [f32; 4]) -> Self {
        Self {
            background: Some(background),
            border_color: None,
            border_width: 0.0,
            corner_radius: corner_radii(0.0),
        }
    }
}

fn visual_quad(
    context: &VisualPrimitiveContext<'_>,
    slot: u64,
    bounds: SceneRect,
    style: VisualQuadStyle,
) -> ScenePrimitive {
    ScenePrimitive {
        id: PrimitiveId {
            node: context.node,
            slot,
        },
        node: context.node,
        bounds,
        transform: context.transform,
        clips: Arc::clone(context.clips),
        opacity: context.opacity,
        z_index: context.z_index,
        document_order: context.document_order,
        kind: ScenePrimitiveKind::Quad {
            background: style.background.map(nana_ui_core::PaintColor::srgb),
            border_color: style.border_color.map(nana_ui_core::PaintColor::srgb),
            border_width: style.border_width,
            corner_radius: style.corner_radius,
            shadow: None,
            surface: QuadSurfacePaint::default(),
        },
    }
}

/// Build a component quad that keeps an explicit authoring-space colour.
/// StandardVisual geometry resolves its slots to sRGB in `VisualQuadStyle`;
/// where the node's authored colour is still that slot's colour, the quad
/// carries the authored value instead, so the GPU converts it to linear scRGB
/// without an sRGB round trip.
fn visual_quad_with_paint(
    context: &VisualPrimitiveContext<'_>,
    slot: u64,
    bounds: SceneRect,
    style: VisualQuadStyle,
    background_color: Option<nana_ui_core::PaintColor>,
    border_color_space: Option<nana_ui_core::PaintColor>,
) -> ScenePrimitive {
    let mut primitive = visual_quad(context, slot, bounds, style);
    if let ScenePrimitiveKind::Quad {
        background,
        border_color,
        ..
    } = &mut primitive.kind
    {
        *background = background_color.or(*background);
        *border_color = border_color_space.or(*border_color);
    }
    primitive
}

/// The one colour a scene slot paints: the authored value when there is one,
/// else the resolved sRGB colour.
pub(crate) fn slot_color(
    paint: Option<nana_ui_core::PaintColor>,
    resolved: Option<[f32; 4]>,
) -> Option<nana_ui_core::PaintColor> {
    paint.or(resolved.map(nana_ui_core::PaintColor::srgb))
}

/// Keep explicit metadata only when it describes the resolved legacy colour
/// used by a component slot.  A StandardVisual often has separate semantic
/// colours for its track, thumb, or indicator, so blindly copying the node's
/// PaintColor would tint the wrong slot.
fn matching_paint_color(
    paint: Option<nana_ui_core::PaintColor>,
    legacy: Option<[f32; 4]>,
) -> Option<nana_ui_core::PaintColor> {
    paint.filter(|paint| legacy == Some(paint.to_srgb()))
}

fn quad_surface_from_style(
    style: &nana_ui_core::LayoutStyle,
    width: f32,
    height: f32,
) -> QuadSurfacePaint {
    let border_fallback = style.paint_colors.border;
    let outline_color = style
        .paint
        .outline
        .is_active()
        .then_some(style.paint.outline.color)
        .flatten();
    QuadSurfacePaint {
        background_image: style.paint.background_image.clone(),
        background_layers: style.paint.background_layers.clone(),
        content_image: style.paint.content_image.clone(),
        mask: style.paint.mask.clone(),
        polygon_clip: style
            .paint
            .clip_path
            .as_ref()
            .and_then(|path| path.resolve_polygon_points(width, height)),
        filter: style.paint.filter.filter(|filter| !filter.is_identity()),
        backdrop_filter: style
            .paint
            .backdrop_filter
            .filter(|filter| filter.is_active()),
        extra_shadows: style
            .paint
            .box_shadows
            .iter()
            .skip(1)
            .copied()
            .map(ComponentElevation::from_box_shadow)
            .collect(),
        outline_width: if style.paint.outline.is_active() {
            style.paint.outline.width
        } else {
            0.0
        },
        // An outline never takes the border's colour: its own, then the
        // current colour as CSS gives an outline without one. A colour set
        // through `PaintStyle::outline` wins over a slot that disagrees.
        outline_color: match outline_color {
            Some(legacy) => slot_color(
                matching_paint_color(style.paint_colors.outline, Some(legacy)),
                Some(legacy),
            ),
            None => style
                .paint_colors
                .outline
                .or(style.paint_colors.color)
                .or(style.color.map(nana_ui_core::PaintColor::srgb)),
        },
        mix_blend: style.paint.mix_blend,
        border_widths: [0.0; 4],
        border_colors: [
            style.paint_colors.border_top.or(border_fallback),
            style.paint_colors.border_right.or(border_fallback),
            style.paint_colors.border_bottom.or(border_fallback),
            style.paint_colors.border_left.or(border_fallback),
        ],
        border_styles: [0; 4],
        border_image: if style.paint.unsupported_border_image {
            None
        } else {
            style.paint.border_image.clone()
        },
    }
}

#[cfg(any(feature = "charts", feature = "graph-canvas", feature = "rich-text"))]
fn visual_stroke(
    context: &VisualPrimitiveContext<'_>,
    slot: u64,
    bounds: SceneRect,
    points: Vec<[f32; 2]>,
    width: f32,
    color: [f32; 4],
) -> ScenePrimitive {
    // Graph/TimeSeries keep `pattern: None`. Vue pathLength stays in SVG markup (resvg).
    ScenePrimitive {
        id: PrimitiveId {
            node: context.node,
            slot,
        },
        node: context.node,
        bounds,
        transform: context.transform,
        clips: Arc::clone(context.clips),
        opacity: context.opacity,
        z_index: context.z_index,
        document_order: context.document_order,
        kind: ScenePrimitiveKind::Stroke {
            points,
            width,
            color,
            widths: Vec::new(),
            cap: StrokeCap::Round,
            pattern: None,
        },
    }
}

/// Markdown inline runs still mark their decorations with box-wide strokes;
/// plain text nodes draw theirs per line and run in the text painter.
#[cfg(feature = "rich-text")]
fn insert_text_decoration_strokes(
    context: &VisualPrimitiveContext<'_>,
    bounds: SceneRect,
    color: [f32; 4],
    deco: nana_ui_core::TextDecorationLine,
    mut sink: impl FnMut(ScenePrimitive),
) {
    let width = 1.0_f32.max(bounds.height * 0.06);
    let mut emit = |slot: u64, y: f32| {
        sink(ScenePrimitive {
            id: PrimitiveId {
                node: context.node,
                slot,
            },
            node: context.node,
            bounds: SceneRect {
                x: bounds.x,
                y: y - width * 0.5,
                width: bounds.width,
                height: width,
            },
            transform: context.transform,
            clips: Arc::clone(context.clips),
            opacity: context.opacity,
            z_index: context.z_index,
            document_order: context.document_order,
            kind: ScenePrimitiveKind::Stroke {
                points: vec![[bounds.x, y], [bounds.x + bounds.width, y]],
                width,
                color,
                widths: Vec::new(),
                cap: StrokeCap::Butt,
                pattern: None,
            },
        });
    };
    if deco.underline {
        emit(12, bounds.y + bounds.height - width);
    }
    if deco.line_through {
        emit(13, bounds.y + bounds.height * 0.5);
    }
}

/// 批次图元共用的批次壳：对逐项 bounds 求并集作为图元 bounds，kind 由
/// 调用方给出（QuadBatch / QuadColorBatch / IconBatch）。
fn batch_primitive(
    context: &VisualPrimitiveContext<'_>,
    slot: u64,
    quad_bounds: Vec<SceneRect>,
    kind: impl FnOnce(Vec<SceneRect>) -> ScenePrimitiveKind,
) -> ScenePrimitive {
    debug_assert!(!quad_bounds.is_empty());
    let bounds = quad_bounds
        .iter()
        .copied()
        .reduce(|left, right| {
            let x = left.x.min(right.x);
            let y = left.y.min(right.y);
            let right_edge = (left.x + left.width).max(right.x + right.width);
            let bottom_edge = (left.y + left.height).max(right.y + right.height);
            SceneRect {
                x,
                y,
                width: right_edge - x,
                height: bottom_edge - y,
            }
        })
        .unwrap_or_default();
    ScenePrimitive {
        id: PrimitiveId {
            node: context.node,
            slot,
        },
        node: context.node,
        bounds,
        transform: context.transform,
        clips: Arc::clone(context.clips),
        opacity: context.opacity,
        z_index: context.z_index,
        document_order: context.document_order,
        kind: kind(quad_bounds),
    }
}

fn visual_quad_batch(
    context: &VisualPrimitiveContext<'_>,
    slot: u64,
    bounds: impl IntoIterator<Item = SceneRect>,
    style: VisualQuadStyle,
) -> ScenePrimitive {
    let quad_bounds = bounds.into_iter().collect::<Vec<_>>();
    batch_primitive(context, slot, quad_bounds, |bounds| {
        ScenePrimitiveKind::QuadBatch {
            bounds,
            background: style.background.map(nana_ui_core::PaintColor::srgb),
            border_color: style.border_color.map(nana_ui_core::PaintColor::srgb),
            border_width: style.border_width,
            corner_radius: style.corner_radius,
            shadow: None,
            surface: QuadSurfacePaint::default(),
        }
    })
}

fn visual_quad_color_batch(
    context: &VisualPrimitiveContext<'_>,
    slot: u64,
    items: impl IntoIterator<Item = (SceneRect, [f32; 4])>,
    style: VisualQuadStyle,
) -> ScenePrimitive {
    let (quad_bounds, colors): (Vec<SceneRect>, Vec<[f32; 4]>) = items.into_iter().unzip();
    debug_assert_eq!(quad_bounds.len(), colors.len());
    batch_primitive(context, slot, quad_bounds, |bounds| {
        ScenePrimitiveKind::QuadColorBatch {
            bounds,
            colors,
            border_color: style.border_color,
            border_width: style.border_width,
            corner_radius: style.corner_radius,
        }
    })
}

/// 锚定浮层面板的共享绘制原语（补全弹层 slot 90 与 hover 浮窗 slot 120
/// 共用）：圆角面板底 + 1px 边框，浮在编辑器内容之上。
fn overlay_panel_primitive(
    context: &VisualPrimitiveContext<'_>,
    slot: u64,
    bounds: SceneRect,
    background: [f32; 4],
    border: [f32; 4],
) -> ScenePrimitive {
    visual_quad(
        context,
        slot,
        bounds,
        VisualQuadStyle {
            background: Some(background),
            border_color: Some(border),
            border_width: 1.0,
            corner_radius: corner_radii(6.0),
        },
    )
}

/// 锚定浮层文本的共享绘制原语：与编辑器文本同族，但不换行（浮层行高
/// 固定），超宽用省略号截断。
#[allow(clippy::too_many_arguments)]
fn overlay_text_primitive(
    id: StableNodeId,
    slot: u64,
    region: &ComponentTextRegion,
    horizontal_alignment: TextHorizontalAlignment,
    node: &ExtractedNode,
    transform: AffineTransform,
    clips: Arc<[ClipRegion]>,
    opacity: f32,
    document_order: usize,
) -> ScenePrimitive {
    let paint_color = node
        .style
        .paint_colors
        .color
        .filter(|paint| region.color.is_none_or(|color| color == paint.to_srgb()));
    ScenePrimitive {
        id: PrimitiveId { node: id, slot },
        node: id,
        bounds: scene_rect(region.bounds),
        transform,
        clips,
        opacity,
        z_index: node.z_index,
        document_order,
        kind: ScenePrimitiveKind::Text {
            content: region.content.clone(),
            color: slot_color(
                paint_color,
                region
                    .color
                    .or_else(|| {
                        node.style
                            .paint_colors
                            .color
                            .map(nana_ui_core::PaintColor::to_srgb)
                    })
                    .or(node.style.color),
            ),
            size: region.font_size,
            weight: region.font_weight,
            family: node.style.font_family.as_deref().map(str::to_owned),
            line_height: node.style.line_height,
            letter_spacing: node.style.letter_spacing,
            wrap: false,
            ellipsis: true,
            max_lines: None,
            shaping: if node.editable {
                TextShaping::Advanced
            } else {
                TextShaping::Auto
            },
            horizontal_alignment,
            vertical_alignment: TextVerticalAlignment::Center,
            spans: Vec::new(),
            text_shadow: None,
            underline: false,
            line_through: false,
            font_features: Vec::new(),
            italic: false,
            wrap_break: nana_ui_core::TextWrapBreak::default(),
            opentype: SceneTextOpenType::default(),
            layout: None,
            rich: None,
            presentation: None,
        },
    }
}

#[cfg(test)]
mod tests;
