//! Backend-neutral markdown blocks and selectable rich text.
//!
//! Runtime owns the block model, source parse, grapheme selection ranges, and
//! leaf projection and native mathematics/diagram drawing. Applications own link
//! handling, image loading/decoding, and clipboard writes. [`NativeMarkdown::from_source`] maps GFM
//! blocks onto [`MarkdownBlock`] / [`MarkdownSpan`]. Scene paint consumes the
//! projected drawing; images and formula SVGs share its normal texture path.
//!
//! [`ComponentView`] projection keeps [`TextContent`] as fallback text and
//! writes [`NativeMarkdownVisual`] / [`SelectableRichTextVisual`]. Selection
//! ranges are half-open grapheme offsets, the same convention as
//! [`TextSelectionSnapshot`].
//!
//! Fenced mermaid, display-math, and code blocks stay on the leaf model.
//! [`AppContext::assemble_markdown`] allocates one hidden text child per fence
//! and attaches [`HighlightRequest`] (`highlight` / [`NativeMarkdown::MERMAID_PRESENTER`] /
//! [`NativeMarkdown::MATH_PRESENTER`]). Those children are identity slots for
//! hosts; they do not emit Scene text. [`NativeMarkdown::project`] does not
//! invent those children.

use std::collections::HashSet;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use nana_ui_core::{
    BackgroundImage, BackgroundImageFit, BackgroundRepeat, SemanticPalette, ThemeMetrics,
};
use pulldown_cmark::{
    Alignment as CmarkAlignment, CodeBlockKind, Event, HeadingLevel, Options, Parser, Tag, TagEnd,
};
use unicode_segmentation::UnicodeSegmentation;

use crate::view_components::project_common;
use crate::{
    AccessibilityRole, AccessibilityState, AppContext, ComponentTextRegion, ComponentView,
    ComputedStyle, DocumentId, Entity, FrameworkError, HighlightRequest, InteractionState,
    LayoutBox, LengthSpec, LineCap, MutationQueue, NodeKind, NodeStyle, SemanticColorRole,
    StableNodeId, TextContent, TextHorizontalAlignment, TextVerticalAlignment, UiWorld,
    WidgetBatchColors, WidgetPaintCtx, WidgetPaintOp, WidgetPaintOutput, WidgetQuad,
    WidgetQuadBatch, WidgetStroke, WidgetText, WidgetVisual,
};

mod span_color;

/// Unit advance used for backend-neutral hit testing. Scene paint owns real
/// glyph metrics.
pub const GRAPHEME_ADVANCE: f32 = nana_ui_core::space::MD;
pub const LINE_HEIGHT: f32 = nana_ui_core::TypographyTokens::DEFAULT.line;
pub const BLOCK_GAP: f32 = 9.0;
/// Inset of a fenced code block's text from its panel edge.
pub const CODE_PANEL_PADDING_X: f32 = nana_ui_core::space::LG;
pub const CODE_PANEL_PADDING_Y: f32 = nana_ui_core::space::MD;
const LIST_INDENT: f32 = nana_ui_core::space::XXL;
const QUOTE_INDENT: f32 = nana_ui_core::space::XL;
/// The title line above a GFM alert's first paragraph.
pub const ALERT_TITLE_HEIGHT: f32 = LINE_HEIGHT;
/// Width of the bar down a GFM alert's left edge.
pub const ALERT_BAR_WIDTH: f32 = 3.0;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct MarkdownSpan {
    pub text: String,
    pub strong: bool,
    pub emphasis: bool,
    pub strikethrough: bool,
    pub code: bool,
    pub inline_math: bool,
    pub link: Option<String>,
    pub image: Option<String>,
    pub image_resource: Option<MarkdownImageResource>,
}

impl MarkdownSpan {
    pub fn plain(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            ..Self::default()
        }
    }
}

/// An application-resolved image. Fetching and decoding remain outside Runtime.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MarkdownImageResource {
    pub source: Arc<str>,
    pub width: u32,
    pub height: u32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MarkdownImage {
    pub source: String,
    pub alt: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MarkdownBlockKind {
    Paragraph,
    Heading(u8),
    Quote,
    ListItem {
        depth: usize,
    },
    /// A paragraph of a GFM alert (`> [!NOTE]` and its kin). The first
    /// paragraph of each alert `opens` it and carries its title line.
    Alert {
        kind: MarkdownAlertKind,
        opens: bool,
    },
}

/// The five GFM alert kinds.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum MarkdownAlertKind {
    Note,
    Tip,
    Important,
    Warning,
    Caution,
}

impl MarkdownAlertKind {
    /// The theme role of the alert's bar and title.
    pub fn color(self) -> SemanticColorRole {
        match self {
            Self::Note => SemanticColorRole::Accent,
            Self::Tip => SemanticColorRole::Success,
            Self::Important => SemanticColorRole::AccentStrong,
            Self::Warning => SemanticColorRole::Warning,
            Self::Caution => SemanticColorRole::Danger,
        }
    }

    /// The alert's title from the document's [`nana_ui_core::FrameworkStrings`]
    /// (`markdown.alert.*`).
    pub fn title(self, strings: &nana_ui_core::FrameworkStrings) -> &Arc<str> {
        match self {
            Self::Note => &strings.markdown_alert_note,
            Self::Tip => &strings.markdown_alert_tip,
            Self::Important => &strings.markdown_alert_important,
            Self::Warning => &strings.markdown_alert_warning,
            Self::Caution => &strings.markdown_alert_caution,
        }
    }

    fn from_cmark(kind: pulldown_cmark::BlockQuoteKind) -> Self {
        use pulldown_cmark::BlockQuoteKind;
        match kind {
            BlockQuoteKind::Note => Self::Note,
            BlockQuoteKind::Tip => Self::Tip,
            BlockQuoteKind::Important => Self::Important,
            BlockQuoteKind::Warning => Self::Warning,
            BlockQuoteKind::Caution => Self::Caution,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum MarkdownTableAlignment {
    #[default]
    Left,
    Center,
    Right,
}

/// Resolved geometry and drawing commands for a [`NativeMarkdown`] component.
#[derive(Clone, Debug, PartialEq)]
pub struct NativeMarkdownLayout {
    pub drawing: crate::MarkdownDrawing,
    pub text: ComponentTextRegion,
    pub selection: Vec<LayoutBox>,
    pub selection_color: [f32; 4],
}

/// Visual payload projected by [`NativeMarkdown`].
#[derive(Clone, Debug, PartialEq)]
pub struct NativeMarkdownVisual {
    pub text: Arc<str>,
    pub blocks: Arc<[MarkdownBlock]>,
    pub selection: Option<(usize, usize)>,
    /// Registered [`crate::TextPresenter`] that colors fenced code.
    pub code_presenter: Arc<str>,
}

/// Resolved geometry and selection highlights for a [`SelectableRichText`] component.
#[derive(Clone, Debug, PartialEq)]
pub struct SelectableRichTextLayout {
    pub text: ComponentTextRegion,
    pub selection: Vec<LayoutBox>,
    pub selection_color: [f32; 4],
}

/// Visual payload projected by [`SelectableRichText`].
#[derive(Clone, Debug, PartialEq)]
pub struct SelectableRichTextVisual {
    pub text: Arc<str>,
    pub selection: Option<(usize, usize)>,
}

fn selectable_text_regions(
    content: LayoutBox,
    text: &Arc<str>,
    style: &ComputedStyle,
    palette: &SemanticPalette,
) -> (ComponentTextRegion, [f32; 4]) {
    let region = ComponentTextRegion {
        bounds: content,
        content: Arc::clone(text).into(),
        color: Some(
            style
                .paint_colors
                .color
                .map(nana_ui_core::PaintColor::to_srgb)
                .or(style.color)
                .unwrap_or_else(|| palette.text.as_rgba_array()),
        ),
        font_size: style.font_size,
        font_weight: style.font_weight,
    };
    (
        region,
        style
            .paint_colors
            .selection_background
            .map(nana_ui_core::PaintColor::to_srgb)
            .or(style.selection_background)
            .unwrap_or_else(|| palette.accent_soft.as_rgba_array()),
    )
}

impl NativeMarkdownVisual {
    /// Resolves `content`-relative markdown drawing commands and selection boxes.
    #[must_use]
    pub fn layout_at(
        &self,
        world: &UiWorld,
        id: StableNodeId,
        content: LayoutBox,
        style: &ComputedStyle,
        palette: &SemanticPalette,
        metrics: ThemeMetrics,
    ) -> NativeMarkdownLayout {
        let (text, selection_color) = selectable_text_regions(content, &self.text, style, palette);
        let geometry = world.markdown_layout(id, &self.blocks, content);
        let style_model = world.node_style_model(id);
        NativeMarkdownLayout {
            drawing: crate::markdown_drawing::document_with_geometry(
                &self.blocks,
                content,
                &geometry,
                &crate::markdown_drawing::MarkdownPaint {
                    metrics,
                    record: &|scene, scale| world.diagram_drawing(scene, scale),
                    color: &|role| style_model.color(role).as_rgba_array(),
                    strings: world.framework_strings(),
                    code_spans: &|language, source| {
                        world.code_presentation(&self.code_presenter, language, source)
                    },
                },
            ),
            text,
            selection: match self.selection {
                Some((start, end)) => geometry
                    .blocks
                    .into_iter()
                    .flat_map(|block| block.graphemes)
                    .filter(|g| g.index >= start.min(end) && g.index < start.max(end))
                    .map(|g| g.bounds)
                    .collect(),
                _ => Vec::new(),
            },
            selection_color,
        }
    }
}

impl WidgetVisual for NativeMarkdownVisual {
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn dyn_eq(&self, other: &dyn std::any::Any) -> bool {
        other.downcast_ref::<Self>() == Some(self)
    }

    fn theme_dependency_mask(&self) -> nana_ui_core::ThemeDependencyMask {
        nana_ui_core::ThemeDependencyMask::COLOR_ANY
            .union(nana_ui_core::ThemeDependencyMask::METRICS)
            .union(nana_ui_core::ThemeDependencyMask::TYPOGRAPHY_ANY)
    }

    fn paint(&self, ctx: &WidgetPaintCtx<'_>) -> WidgetPaintOutput {
        self.layout_at(
            ctx.world,
            ctx.id,
            ctx.content,
            ctx.style,
            &ctx.style_model.palette,
            ctx.style_model.metrics,
        )
        .to_widget_paint(ctx.style)
    }
}

impl NativeMarkdownLayout {
    /// Lowers this resolved layout into node-local widget paint operations.
    #[must_use]
    pub fn to_widget_paint(&self, style: &ComputedStyle) -> WidgetPaintOutput {
        let mut out = WidgetPaintOutput::new();
        out.owns_node_text = true;
        out.suppress_root_surface = true;

        if !self.selection.is_empty() {
            out.push(
                1,
                WidgetPaintOp::QuadBatch(WidgetQuadBatch {
                    bounds: crate::widget_visual::union_layout_rects(&self.selection),
                    rects: Arc::from(self.selection.as_slice()),
                    colors: WidgetBatchColors::Uniform(self.selection_color),
                    fill_paint: style.paint_colors.selection_background,
                    border: None,
                    corner_radius: [0.0; 4],
                }),
            );
        }
        let color = self.text.color.unwrap_or([0.8, 0.8, 0.8, 1.0]);
        for (index, command) in self.drawing.commands.iter().enumerate() {
            let base_slot = (index as u64).saturating_mul(4);
            let slot = 1025u64.saturating_add(base_slot);
            match command {
                crate::MarkdownDrawingCommand::Diagram { bounds, drawing } => {
                    out.push(
                        slot,
                        WidgetPaintOp::Vector {
                            bounds: *bounds,
                            recording: Arc::clone(&drawing.recording),
                        },
                    );
                }
                crate::MarkdownDrawingCommand::Image { bounds, source } => {
                    let mut image =
                        BackgroundImage::url_with_fit(source.as_ref(), BackgroundImageFit::Contain);
                    if let BackgroundImage::Url { repeat, .. } = &mut image {
                        *repeat = BackgroundRepeat::NoRepeat;
                    }
                    out.push(
                        slot,
                        WidgetPaintOp::Quad(WidgetQuad {
                            rect: *bounds,
                            content_image: Some(image),
                            ..WidgetQuad::default()
                        }),
                    );
                }
                crate::MarkdownDrawingCommand::Svg { bounds, source } => {
                    let ink = format!(
                        "rgb({},{},{})",
                        (color[0].clamp(0.0, 1.0) * 255.0).round() as u8,
                        (color[1].clamp(0.0, 1.0) * 255.0).round() as u8,
                        (color[2].clamp(0.0, 1.0) * 255.0).round() as u8
                    );
                    let svg = source.replace("#010203", &ink).replace("rgb(1,2,3)", &ink);
                    let mut url = String::from("data:image/svg+xml,");
                    use std::fmt::Write as _;
                    for byte in svg.bytes() {
                        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'~')
                        {
                            url.push(byte as char);
                        } else {
                            let _ = write!(url, "%{byte:02X}");
                        }
                    }
                    let mut image = BackgroundImage::url_with_fit(url, BackgroundImageFit::Contain);
                    if let BackgroundImage::Url { repeat, .. } = &mut image {
                        *repeat = BackgroundRepeat::NoRepeat;
                    }
                    out.push(
                        slot,
                        WidgetPaintOp::Quad(WidgetQuad {
                            rect: *bounds,
                            content_image: Some(image),
                            ..WidgetQuad::default()
                        }),
                    );
                }
                crate::MarkdownDrawingCommand::Text {
                    bounds,
                    text,
                    size,
                    weight,
                    italic,
                    line_through,
                    underline,
                    code,
                    color: run_color,
                } => {
                    let color = run_color.unwrap_or(color);
                    if *code {
                        let mut tint = color;
                        tint[3] *= 0.10;
                        out.push(
                            1024u64.saturating_add(base_slot),
                            WidgetPaintOp::Quad(WidgetQuad {
                                rect: *bounds,
                                fill: Some(tint),
                                corner_radius: [2.0; 4],
                                ..WidgetQuad::default()
                            }),
                        );
                    }
                    let width = 1.0_f32.max(bounds.height * 0.06);
                    if *underline {
                        let y = bounds.y + bounds.height - width;
                        out.push(
                            1026u64.saturating_add(base_slot),
                            WidgetPaintOp::Stroke(WidgetStroke {
                                points: vec![[bounds.x, y], [bounds.x + bounds.width, y]],
                                width,
                                color,
                                cap: LineCap::Butt,
                                dash: None,
                                snap_hairline: false,
                            }),
                        );
                    }
                    if *line_through {
                        let y = bounds.y + bounds.height * 0.5;
                        out.push(
                            1027u64.saturating_add(base_slot),
                            WidgetPaintOp::Stroke(WidgetStroke {
                                points: vec![[bounds.x, y], [bounds.x + bounds.width, y]],
                                width,
                                color,
                                cap: LineCap::Butt,
                                dash: None,
                                snap_hairline: false,
                            }),
                        );
                    }
                    out.push(
                        slot,
                        WidgetPaintOp::Text(WidgetText {
                            bounds: *bounds,
                            content: text.clone().into(),
                            color: Some(color),
                            paint_color: crate::widget_visual::matching_paint_color(
                                style.paint_colors.color,
                                color,
                            ),
                            font_size: *size,
                            font_weight: Some(*weight),
                            horizontal_alignment: TextHorizontalAlignment::Start,
                            vertical_alignment: TextVerticalAlignment::Top,
                            ellipsis: false,
                            wrap: false,
                            max_lines: None,
                            italic: Some(*italic),
                            underline: Some(*underline),
                            line_through: Some(*line_through),
                            suppress_spans: true,
                            preserve_lines: None,
                            unstyled_glyph: false,
                            line_height: None,
                            retained_text: crate::WidgetRetainedText::None,
                            tabular_nums: false,
                            ..Default::default()
                        }),
                    );
                }
                crate::MarkdownDrawingCommand::Line {
                    points,
                    width,
                    color: line_color,
                } => {
                    out.push(
                        slot,
                        WidgetPaintOp::Stroke(WidgetStroke {
                            points: points.clone(),
                            width: *width,
                            color: line_color.unwrap_or(color),
                            cap: LineCap::Round,
                            dash: None,
                            snap_hairline: false,
                        }),
                    );
                }
                crate::MarkdownDrawingCommand::Panel { bounds, radius } => {
                    let mut fill = color;
                    fill[3] *= 0.06;
                    let mut edge = color;
                    edge[3] *= 0.12;
                    out.push(
                        slot,
                        WidgetPaintOp::Quad(WidgetQuad {
                            rect: *bounds,
                            fill: Some(fill),
                            border: Some((edge, 1.0)),
                            corner_radius: [*radius; 4],
                            ..WidgetQuad::default()
                        }),
                    );
                }
                crate::MarkdownDrawingCommand::Box { bounds, radius } => {
                    out.push(
                        slot,
                        WidgetPaintOp::Quad(WidgetQuad {
                            rect: *bounds,
                            fill: None,
                            border: Some((color, 1.0)),
                            corner_radius: [*radius; 4],
                            ..WidgetQuad::default()
                        }),
                    );
                }
            }
        }
        out
    }
}

impl SelectableRichTextVisual {
    /// Resolves `content`-relative rich-text region and selection highlights.
    #[must_use]
    pub fn layout_at(
        &self,
        world: &UiWorld,
        id: StableNodeId,
        content: LayoutBox,
        style: &ComputedStyle,
        palette: &SemanticPalette,
    ) -> SelectableRichTextLayout {
        let selection =
            world.selectable_rich_text_highlights(id, content, &self.text, self.selection);
        let (text, selection_color) = selectable_text_regions(content, &self.text, style, palette);
        SelectableRichTextLayout {
            text,
            selection,
            selection_color,
        }
    }
}

impl WidgetVisual for SelectableRichTextVisual {
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn dyn_eq(&self, other: &dyn std::any::Any) -> bool {
        other.downcast_ref::<Self>() == Some(self)
    }

    fn theme_dependency_mask(&self) -> nana_ui_core::ThemeDependencyMask {
        nana_ui_core::ThemeDependencyMask::COLOR_ANY
            .union(nana_ui_core::ThemeDependencyMask::METRICS)
            .union(nana_ui_core::ThemeDependencyMask::TYPOGRAPHY_ANY)
    }

    fn paint(&self, ctx: &WidgetPaintCtx<'_>) -> WidgetPaintOutput {
        let layout = self.layout_at(
            ctx.world,
            ctx.id,
            ctx.content,
            ctx.style,
            &ctx.style_model.palette,
        );
        let mut out = WidgetPaintOutput::new();
        out.owns_node_text = true;
        out.suppress_root_surface = true;

        if !layout.selection.is_empty() {
            out.push(
                1,
                WidgetPaintOp::QuadBatch(WidgetQuadBatch {
                    bounds: crate::widget_visual::union_layout_rects(&layout.selection),
                    rects: Arc::from(layout.selection),
                    colors: WidgetBatchColors::Uniform(layout.selection_color),
                    fill_paint: ctx.style.paint_colors.selection_background,
                    border: None,
                    corner_radius: [0.0; 4],
                }),
            );
        }
        let mut text = crate::widget_visual::widget_text_region(
            &layout.text,
            ctx,
            TextHorizontalAlignment::Start,
            false,
        );
        text.vertical_alignment = TextVerticalAlignment::Top;
        text.wrap = true;
        out.push(2, WidgetPaintOp::Text(text));
        out
    }
}

impl UiWorld {
    /// Resolves `id`'s markdown drawing commands and selection geometry.
    #[must_use]
    pub fn native_markdown_layout(&self, id: StableNodeId) -> Option<NativeMarkdownLayout> {
        let visual = self.widget_visual_as::<NativeMarkdownVisual>(id)?;
        let content = self.component_content_box(id)?;
        let style = self.computed_style(id)?;
        let style_model = self.node_style_model(id);
        Some(visual.layout_at(
            self,
            id,
            content,
            style,
            &style_model.palette,
            style_model.metrics,
        ))
    }

    /// Where the mermaid diagram of `id`'s block `block_index` shows, at
    /// what natural size and scale. `None` when that block is not a diagram
    /// the engine drew (unreadable source shows as code) or `id` is not a
    /// laid-out [`NativeMarkdown`].
    #[must_use]
    pub fn markdown_diagram_placement(
        &self,
        id: StableNodeId,
        block_index: usize,
    ) -> Option<MarkdownDiagramPlacement> {
        let visual = self.widget_visual_as::<NativeMarkdownVisual>(id)?;
        let content = self.component_content_box(id)?;
        self.markdown_layout(id, &visual.blocks, content)
            .blocks
            .get(block_index)?
            .diagram_placement()
    }

    /// Resolves `id`'s selectable rich-text region and selection highlights.
    #[must_use]
    pub fn selectable_rich_text_paint_layout(
        &self,
        id: StableNodeId,
    ) -> Option<SelectableRichTextLayout> {
        let visual = self.widget_visual_as::<SelectableRichTextVisual>(id)?;
        let content = self.component_content_box(id)?;
        let style = self.computed_style(id)?;
        let style_model = self.node_style_model(id);
        Some(visual.layout_at(self, id, content, style, &style_model.palette))
    }

    /// The horizontal `nana-text` layout a `SelectableRichText` showing
    /// `text` was measured with and is painted from. `None` when a host shaper
    /// measured it, it is a vertical column, or it holds other text.
    fn selectable_rich_text_retained(
        &self,
        id: StableNodeId,
        text: &str,
    ) -> Option<&Arc<nana_text::TextLayout>> {
        let (_, layout) = self.text_layout(id)?;
        (!layout.is_vertical() && self.text(id) == Some(text)).then_some(layout)
    }

    /// Hit geometry of a `SelectableRichText` at content box `bounds`, where
    /// its text is painted; the fixed-advance estimate without a retained
    /// layout.
    pub(crate) fn selectable_rich_text_layout(
        &self,
        id: StableNodeId,
        view: &SelectableRichText,
        bounds: LayoutBox,
    ) -> RichTextGeometry {
        match self.selectable_rich_text_retained(id, &view.plain_text()) {
            Some(layout) => layout_rich_spans_shaped(view.spans(), bounds, layout),
            None => view.layout(bounds),
        }
    }

    /// Highlight of a `SelectableRichText` selection (grapheme offsets): the
    /// selected part of each painted line; the whole content box without a
    /// retained layout.
    pub(crate) fn selectable_rich_text_highlights(
        &self,
        id: StableNodeId,
        content: LayoutBox,
        text: &str,
        selection: Option<(usize, usize)>,
    ) -> Vec<LayoutBox> {
        let Some((anchor, focus)) = selection else {
            return Vec::new();
        };
        let Some(layout) = self.selectable_rich_text_retained(id, text) else {
            return vec![content];
        };
        let byte = |grapheme: usize| {
            text.grapheme_indices(true)
                .nth(grapheme)
                .map_or(text.len(), |(offset, _)| offset)
        };
        layout
            .selection_rects(byte(anchor.min(focus))..byte(anchor.max(focus)))
            .into_iter()
            .map(|rect| text_rect_at(content, rect))
            .collect()
    }

    /// Markdown geometry measured by the engine that shaped this world, the
    /// one the painter draws the runs with. A world no engine has shaped (a
    /// host shaper without one) keeps the em estimate.
    pub(crate) fn markdown_layout(
        &self,
        id: StableNodeId,
        blocks: &[MarkdownBlock],
        bounds: LayoutBox,
    ) -> MarkdownGeometry {
        let (Some(style), Some(engine)) =
            (self.computed_style(id), self.paint_text_engine.as_ref())
        else {
            return layout_markdown(blocks, bounds);
        };
        // What the advances depend on besides the text: the engine, its face
        // set and the node's family and spacing. Diagrams cache under it.
        let key = {
            use std::hash::{Hash, Hasher};
            let mut hasher = std::collections::hash_map::DefaultHasher::new();
            (Arc::as_ptr(engine) as usize).hash(&mut hasher);
            {
                use nana_text::TextEngine as _;
                nana_text::lock_text_engine(engine)
                    .font_generation()
                    .get()
                    .hash(&mut hasher);
            }
            style.font_family.hash(&mut hasher);
            style.letter_spacing.to_bits().hash(&mut hasher);
            hasher.finish()
        };
        layout_markdown_measured(
            blocks,
            bounds,
            Some(&|text, size, weight, italic| {
                let mut measured = style.clone();
                measured.font_size = size;
                measured.font_weight = Some(weight);
                measured.italic = italic;
                // A markdown run is one unwrapped horizontal line.
                measured.writing_mode = nana_ui_core::WritingModeSpec::HorizontalTb;
                crate::text_engine_shaper::grapheme_advances(engine, text, &measured)
            }),
            key,
        )
    }

    /// A laid-out diagram recorded against the installed theme, reused
    /// until the palette epoch moves.
    pub(crate) fn diagram_drawing(
        &self,
        scene: &Arc<nana_ui_diagram::DiagramScene>,
        scale: f32,
    ) -> crate::markdown_diagram::DiagramDrawing {
        let epoch = self.palette_epoch;
        let mut held = self.diagram_recordings.borrow_mut();
        held.retain(|(e, _)| *e == epoch);
        if let Some((_, drawing)) = held
            .iter()
            .find(|(_, d)| Arc::ptr_eq(&d.scene, scene) && d.scale == scale)
        {
            return drawing.clone();
        }
        let drawing = crate::markdown_diagram::record(scene, self.theme(), scale);
        if held.len() >= 64 {
            held.remove(0);
        }
        held.push((epoch, drawing.clone()));
        drawing
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct MarkdownTable {
    pub alignments: Vec<MarkdownTableAlignment>,
    pub header: Vec<Vec<MarkdownSpan>>,
    pub rows: Vec<Vec<Vec<MarkdownSpan>>>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MarkdownBlock {
    Text {
        kind: MarkdownBlockKind,
        spans: Vec<MarkdownSpan>,
    },
    Code {
        language: Option<String>,
        source: String,
    },
    DisplayMath(String),
    Mermaid(String),
    Table(MarkdownTable),
    Rule,
}

impl MarkdownBlock {
    pub fn paragraph(spans: impl IntoIterator<Item = MarkdownSpan>) -> Self {
        Self::Text {
            kind: MarkdownBlockKind::Paragraph,
            spans: spans.into_iter().collect(),
        }
    }

    pub fn heading(level: u8, spans: impl IntoIterator<Item = MarkdownSpan>) -> Self {
        Self::Text {
            kind: MarkdownBlockKind::Heading(level),
            spans: spans.into_iter().collect(),
        }
    }

    pub fn code(language: Option<&str>, source: impl Into<String>) -> Self {
        Self::Code {
            language: language.map(str::to_owned),
            source: source.into(),
        }
    }

    pub fn language(&self) -> Option<&str> {
        match self {
            Self::Code { language, .. } => language.as_deref(),
            _ => None,
        }
    }

    /// Fence body for mermaid, display-math, and code blocks.
    pub fn fence_source(&self) -> Option<&str> {
        match self {
            Self::Code { source, .. } | Self::DisplayMath(source) | Self::Mermaid(source) => {
                Some(source.as_str())
            }
            _ => None,
        }
    }

    /// Highlight intent for a fenced code block. Parent wiring attaches this
    /// to a child text node; the markdown leaf does not allocate that child.
    pub fn highlight_request(&self) -> Option<HighlightRequest> {
        match self {
            Self::Code {
                language: Some(language),
                ..
            } if !language.is_empty() => Some(HighlightRequest::highlight(language.as_str())),
            _ => None,
        }
    }

    /// Presenter identity for a fence child. Code uses `"highlight"` plus the
    /// language; mermaid/math use stable presenter names and the fence source
    /// as `language` so hosts can bind a painter.
    pub fn fence_highlight_request(&self) -> Option<HighlightRequest> {
        match self {
            Self::Code { .. } => self.highlight_request(),
            Self::Mermaid(source) => Some(HighlightRequest::new(
                NativeMarkdown::MERMAID_PRESENTER,
                source.as_str(),
            )),
            Self::DisplayMath(source) => Some(HighlightRequest::new(
                NativeMarkdown::MATH_PRESENTER,
                source.as_str(),
            )),
            _ => None,
        }
    }

    fn fence_child(&self) -> Option<MarkdownFenceChild> {
        Some(MarkdownFenceChild {
            source: self.fence_source()?.to_owned(),
            highlight: self.fence_highlight_request(),
            diagram: match self {
                Self::Mermaid(source) => crate::markdown_diagram::describe(source),
                _ => None,
            },
        })
    }
}

#[derive(Clone, Debug)]
pub struct NativeMarkdown {
    source: Option<Arc<str>>,
    blocks: Vec<MarkdownBlock>,
    selection: TextSelectionGroup,
    fence_children: Vec<StableNodeId>,
    code_presenter: Arc<str>,
    pub style: NodeStyle,
}

/// Every field `project` reads takes part: `AppContext::update_component`
/// skips projection when a component compares equal to what it was. The
/// selection group is shared state, so it compares by identity.
impl PartialEq for NativeMarkdown {
    fn eq(&self, other: &Self) -> bool {
        self.blocks == other.blocks
            && self.source == other.source
            && self.selection.id == other.selection.id
            && self.fence_children == other.fence_children
            && self.code_presenter == other.code_presenter
            && self.style == other.style
    }
}

impl Default for NativeMarkdown {
    fn default() -> Self {
        Self::new()
    }
}

impl NativeMarkdown {
    /// Presenter name hosts bind for mermaid fence children.
    pub const MERMAID_PRESENTER: &'static str = "mermaid";
    /// Presenter name hosts bind for display-math fence children.
    pub const MATH_PRESENTER: &'static str = "math";

    pub fn new() -> Self {
        Self {
            source: None,
            blocks: Vec::new(),
            selection: TextSelectionGroup::new(),
            fence_children: Vec::new(),
            code_presenter: Arc::from(crate::HIGHLIGHT_PRESENTER),
            style: NodeStyle::default(),
        }
    }

    pub fn from_blocks(blocks: impl IntoIterator<Item = MarkdownBlock>) -> Self {
        Self {
            source: None,
            blocks: blocks.into_iter().collect(),
            selection: TextSelectionGroup::new(),
            fence_children: Vec::new(),
            code_presenter: Arc::from(crate::HIGHLIGHT_PRESENTER),
            style: NodeStyle::default(),
        }
    }

    /// Parse CommonMark / GFM source into native blocks.
    pub fn from_source(source: &str) -> Self {
        Self::parse(source)
    }

    /// Alias of [`Self::from_source`].
    pub fn parse(source: &str) -> Self {
        let mut parser = MarkdownParser::default();
        let options = Options::ENABLE_GFM
            | Options::ENABLE_TABLES
            | Options::ENABLE_STRIKETHROUGH
            | Options::ENABLE_TASKLISTS
            | Options::ENABLE_MATH;
        let normalized = normalize_math_delimiters(source);
        for event in Parser::new_ext(&normalized, options) {
            parser.push(event);
        }
        let mut markdown = parser.finish();
        markdown.source = Some(Arc::from(source));
        markdown
    }

    #[allow(dead_code)]
    pub(crate) fn source(&self) -> Option<&str> {
        self.source.as_deref()
    }

    pub fn blocks(&self) -> &[MarkdownBlock] {
        &self.blocks
    }

    /// Text children allocated by [`AppContext::assemble_markdown`] for fence
    /// blocks. Empty until assembly runs.
    pub fn fence_children(&self) -> &[StableNodeId] {
        &self.fence_children
    }

    pub fn selection_group(&self) -> &TextSelectionGroup {
        &self.selection
    }

    pub fn group_id(&self) -> TextSelectionGroupId {
        self.selection.id()
    }

    pub fn style(mut self, style: NodeStyle) -> Self {
        self.style = style;
        self
    }

    /// Names the registered [`crate::TextPresenter`] that colors fenced code
    /// (default [`crate::HIGHLIGHT_PRESENTER`]). Each fence with a language
    /// is handed to it as text plus a [`HighlightRequest`] carrying the
    /// language; the spans it returns paint the fence's runs in their theme
    /// roles. With no such presenter registered fences stay one color.
    /// Register a closure with [`crate::TextPresenterFn`]; under the
    /// `syntax-highlighting` feature `AppContext::new` already installs
    /// the syntect presenter as `"highlight"`.
    pub fn code_presenter(mut self, presenter: impl Into<Arc<str>>) -> Self {
        self.code_presenter = presenter.into();
        self
    }

    /// The presenter [`Self::code_presenter`] named.
    pub fn code_presenter_name(&self) -> &str {
        &self.code_presenter
    }

    pub fn plain_text(&self) -> String {
        self.blocks
            .iter()
            .map(markdown_block_plain_text)
            .filter(|value| !value.is_empty())
            .collect::<Vec<_>>()
            .join("\n\n")
    }

    pub fn images(&self) -> Vec<MarkdownImage> {
        let mut images = Vec::new();
        for block in &self.blocks {
            match block {
                MarkdownBlock::Text { spans, .. } => collect_markdown_images(spans, &mut images),
                MarkdownBlock::Table(table) => {
                    for cell in table.header.iter().chain(table.rows.iter().flatten()) {
                        collect_markdown_images(cell, &mut images);
                    }
                }
                _ => {}
            }
        }
        images
    }

    /// Publishes an already resolved image to every matching occurrence.
    pub fn resolve_image(
        &mut self,
        source: &str,
        resource: impl Into<Arc<str>>,
        width: u32,
        height: u32,
    ) -> bool {
        if width == 0 || height == 0 {
            return false;
        }
        let resource = MarkdownImageResource {
            source: resource.into(),
            width,
            height,
        };
        let mut changed = false;
        let mut resolve = |span: &mut MarkdownSpan| {
            if span.image.as_deref() == Some(source)
                && span.image_resource.as_ref() != Some(&resource)
            {
                span.image_resource = Some(resource.clone());
                changed = true;
            }
        };
        for block in &mut self.blocks {
            match block {
                MarkdownBlock::Text { spans, .. } => spans.iter_mut().for_each(&mut resolve),
                MarkdownBlock::Table(table) => table
                    .header
                    .iter_mut()
                    .chain(table.rows.iter_mut().flatten())
                    .flatten()
                    .for_each(&mut resolve),
                _ => {}
            }
        }
        changed
    }

    pub fn code_highlights(&self) -> Vec<(usize, HighlightRequest)> {
        self.blocks
            .iter()
            .enumerate()
            .filter_map(|(index, block)| block.highlight_request().map(|request| (index, request)))
            .collect()
    }

    pub fn selected_text(&self) -> Option<String> {
        self.selection.snapshot().map(|snapshot| snapshot.text)
    }

    pub fn selection_snapshot(&self) -> Option<TextSelectionSnapshot> {
        self.selection.snapshot()
    }

    pub fn copy_snapshot(&self) -> Option<TextSelectionSnapshot> {
        self.selection.snapshot()
    }

    pub fn clear_selection(&self) {
        self.selection.clear();
    }

    pub fn layout(&self, bounds: LayoutBox) -> MarkdownGeometry {
        layout_markdown(&self.blocks, bounds)
    }

    pub fn drawing(&self, bounds: LayoutBox) -> crate::MarkdownDrawing {
        crate::markdown_drawing::document(&self.blocks, bounds)
    }

    pub fn pointer_down(&self, x: f32, y: f32, bounds: LayoutBox) -> bool {
        let geometry = self.layout(bounds);
        self.pointer_down_with_geometry(x, y, &geometry)
    }

    pub(crate) fn pointer_down_with_geometry(
        &self,
        x: f32,
        y: f32,
        geometry: &MarkdownGeometry,
    ) -> bool {
        self.selection.begin(&geometry.run, x, y)
    }

    pub fn pointer_move(&self, x: f32, y: f32, bounds: LayoutBox) -> bool {
        let geometry = self.layout(bounds);
        self.pointer_move_with_geometry(x, y, &geometry)
    }

    pub(crate) fn pointer_move_with_geometry(
        &self,
        x: f32,
        y: f32,
        geometry: &MarkdownGeometry,
    ) -> bool {
        self.selection.drag(&geometry.run, x, y)
    }

    pub fn pointer_up(&self, x: f32, y: f32, bounds: LayoutBox) -> Option<RichTextEvent> {
        let geometry = self.layout(bounds);
        self.pointer_up_with_geometry(x, y, &geometry)
    }

    pub(crate) fn pointer_up_with_geometry(
        &self,
        x: f32,
        y: f32,
        geometry: &MarkdownGeometry,
    ) -> Option<RichTextEvent> {
        let event = self.selection.finish(&geometry.run, x, y)?;
        if let RichTextEvent::SelectionChanged(snapshot) = &event
            && snapshot
                .as_ref()
                .is_none_or(|value| value.start == value.end)
            && let Some(placement) = geometry
                .blocks
                .iter()
                .filter_map(MarkdownBlockGeometry::diagram_placement)
                .find(|placement| placement.bounds.contains(x, y))
            && let Some(MarkdownBlock::Mermaid(source)) = self.blocks.get(placement.block_index)
        {
            return Some(RichTextEvent::DiagramActivated(MarkdownDiagram {
                block_index: placement.block_index,
                source: Arc::from(source.as_str()),
            }));
        }
        if matches!(event, RichTextEvent::LinkActivated(_)) {
            let hit = geometry
                .run
                .graphemes
                .iter()
                .find(|item| item.bounds.contains(x, y));
            if let Some(hit) = hit {
                let span = match self.blocks.get(hit.block_index) {
                    Some(MarkdownBlock::Text { spans, .. }) => spans.get(hit.span_index),
                    Some(MarkdownBlock::Table(table)) => table
                        .header
                        .iter()
                        .chain(table.rows.iter().flatten())
                        .flatten()
                        .nth(hit.span_index),
                    _ => None,
                };
                if let Some(span) = span
                    && let Some(source) = &span.image
                {
                    return Some(RichTextEvent::ImageActivated(MarkdownImage {
                        source: source.clone(),
                        alt: span.text.clone(),
                    }));
                }
            }
        }
        Some(event)
    }

    pub fn link_at(&self, x: f32, y: f32, bounds: LayoutBox) -> Option<Arc<str>> {
        self.layout(bounds).run.link_at(x, y)
    }
}

#[derive(Clone, Copy, Debug)]
enum PendingTextKind {
    Paragraph,
    Heading(u8),
    ListItem { depth: usize },
}

#[derive(Debug, Default)]
struct MarkdownParser {
    blocks: Vec<MarkdownBlock>,
    spans: Vec<MarkdownSpan>,
    pending_kind: Option<PendingTextKind>,
    strong_depth: usize,
    emphasis_depth: usize,
    strike_depth: usize,
    link: Option<String>,
    image: Option<String>,
    image_start: usize,
    code_block: Option<(Option<String>, String)>,
    quote_depth: usize,
    /// One entry per open block quote: the alert it is, if it is one.
    alerts: Vec<Option<MarkdownAlertKind>>,
    /// The innermost alert has not given a paragraph its title line yet.
    alert_untitled: bool,
    lists: Vec<Option<u64>>,
    table: Option<MarkdownTableBuilder>,
    table_header: bool,
    table_row: Vec<Vec<MarkdownSpan>>,
    in_table_cell: bool,
}

#[derive(Debug)]
struct MarkdownTableBuilder {
    alignments: Vec<MarkdownTableAlignment>,
    header: Option<Vec<Vec<MarkdownSpan>>>,
    rows: Vec<Vec<Vec<MarkdownSpan>>>,
}

impl MarkdownParser {
    fn push(&mut self, event: Event<'_>) {
        if self.code_block.is_some() {
            match event {
                Event::Text(value) | Event::Code(value) => {
                    self.code_block.as_mut().unwrap().1.push_str(&value);
                }
                Event::End(TagEnd::CodeBlock) => self.finish_code_block(),
                _ => {}
            }
            return;
        }

        match event {
            Event::Start(tag) => self.start(tag),
            Event::End(tag) => self.end(tag),
            Event::Text(value) => self.push_span(value.into_string(), false, false),
            Event::Code(value) => self.push_span(value.into_string(), true, false),
            Event::InlineMath(value) => self.push_span(value.into_string(), false, true),
            Event::DisplayMath(value) => {
                self.flush_text();
                self.blocks
                    .push(MarkdownBlock::DisplayMath(value.into_string()));
            }
            Event::SoftBreak | Event::HardBreak => self.push_span("\n".to_owned(), false, false),
            Event::Rule => {
                self.flush_text();
                self.blocks.push(MarkdownBlock::Rule);
            }
            Event::TaskListMarker(done) => {
                self.push_span(if done { "☑ " } else { "☐ " }.to_owned(), false, false)
            }
            Event::Html(value) | Event::InlineHtml(value) => {
                self.push_span(value.into_string(), false, false)
            }
            Event::FootnoteReference(value) => {
                self.push_span(format!("[{}]", value.into_string()), false, false)
            }
        }
    }

    fn start(&mut self, tag: Tag<'_>) {
        match tag {
            Tag::Paragraph => {
                self.pending_kind.get_or_insert(PendingTextKind::Paragraph);
            }
            Tag::Heading { level, .. } => {
                self.flush_text();
                self.pending_kind = Some(PendingTextKind::Heading(heading_level(level)));
            }
            Tag::BlockQuote(kind) => {
                self.flush_text();
                self.quote_depth += 1;
                let alert = kind.map(MarkdownAlertKind::from_cmark);
                self.alert_untitled |= alert.is_some();
                self.alerts.push(alert);
            }
            Tag::CodeBlock(kind) => {
                self.flush_text();
                let language = match kind {
                    CodeBlockKind::Indented => None,
                    CodeBlockKind::Fenced(value) => value
                        .split_ascii_whitespace()
                        .next()
                        .filter(|value| !value.is_empty())
                        .map(str::to_owned),
                };
                self.code_block = Some((language, String::new()));
            }
            Tag::List(start) => self.lists.push(start),
            Tag::Item => {
                self.flush_text();
                self.pending_kind = Some(PendingTextKind::ListItem {
                    depth: self.lists.len().max(1),
                });
                let prefix = match self.lists.last_mut() {
                    Some(Some(next)) => {
                        let prefix = format!("{next}. ");
                        *next = next.saturating_add(1);
                        prefix
                    }
                    _ => "• ".to_owned(),
                };
                self.push_span(prefix, false, false);
            }
            Tag::Emphasis => self.emphasis_depth += 1,
            Tag::Strong => self.strong_depth += 1,
            Tag::Strikethrough => self.strike_depth += 1,
            Tag::Link { dest_url, .. } => {
                self.link = Some(dest_url.into_string());
            }
            Tag::Image { dest_url, .. } => {
                self.image_start = self.spans.len();
                self.image = Some(dest_url.into_string());
            }
            Tag::Table(alignments) => {
                self.flush_text();
                self.table_row.clear();
                self.table = Some(MarkdownTableBuilder {
                    alignments: alignments.into_iter().map(table_alignment).collect(),
                    header: None,
                    rows: Vec::new(),
                });
            }
            Tag::TableHead => self.table_header = true,
            Tag::TableRow => self.table_row.clear(),
            Tag::TableCell => {
                self.spans.clear();
                self.in_table_cell = true;
            }
            Tag::HtmlBlock
            | Tag::FootnoteDefinition(_)
            | Tag::MetadataBlock(_)
            | Tag::DefinitionList
            | Tag::DefinitionListTitle
            | Tag::DefinitionListDefinition
            | Tag::Superscript
            | Tag::Subscript => {}
        }
    }

    fn end(&mut self, tag: TagEnd) {
        match tag {
            TagEnd::Paragraph | TagEnd::Heading(_) | TagEnd::Item => self.flush_text(),
            TagEnd::BlockQuote(_) => {
                self.flush_text();
                // An alert with no paragraph of its own still shows its title.
                if let Some(Some(kind)) = self.alerts.pop()
                    && std::mem::take(&mut self.alert_untitled)
                {
                    self.blocks.push(MarkdownBlock::Text {
                        kind: MarkdownBlockKind::Alert { kind, opens: true },
                        spans: Vec::new(),
                    });
                }
                self.quote_depth = self.quote_depth.saturating_sub(1);
            }
            TagEnd::List(_) => {
                self.flush_text();
                self.lists.pop();
            }
            TagEnd::Emphasis => self.emphasis_depth = self.emphasis_depth.saturating_sub(1),
            TagEnd::Strong => self.strong_depth = self.strong_depth.saturating_sub(1),
            TagEnd::Strikethrough => self.strike_depth = self.strike_depth.saturating_sub(1),
            TagEnd::Link => self.link = None,
            TagEnd::Image => {
                if self.spans.len() == self.image_start {
                    self.spans.push(MarkdownSpan {
                        image: self.image.clone(),
                        ..Default::default()
                    });
                }
                self.image = None;
            }
            TagEnd::TableCell => {
                self.table_row.push(std::mem::take(&mut self.spans));
                self.in_table_cell = false;
            }
            TagEnd::TableRow => self.finish_table_row(),
            TagEnd::TableHead => {
                self.finish_table_row();
                self.table_header = false;
            }
            TagEnd::Table => {
                self.finish_table_row();
                self.finish_table();
            }
            TagEnd::CodeBlock
            | TagEnd::HtmlBlock
            | TagEnd::FootnoteDefinition
            | TagEnd::MetadataBlock(_)
            | TagEnd::DefinitionList
            | TagEnd::DefinitionListTitle
            | TagEnd::DefinitionListDefinition
            | TagEnd::Superscript
            | TagEnd::Subscript => {}
        }
    }

    fn push_span(&mut self, value: String, code: bool, inline_math: bool) {
        if value.is_empty() {
            return;
        }
        if self.pending_kind.is_none() && !self.in_table_cell {
            self.pending_kind = Some(PendingTextKind::Paragraph);
        }
        let next = MarkdownSpan {
            text: value,
            strong: self.strong_depth > 0,
            emphasis: self.emphasis_depth > 0,
            strikethrough: self.strike_depth > 0,
            code,
            inline_math,
            link: self.link.clone(),
            image: self.image.clone(),
            image_resource: None,
        };
        if let Some(last) = self.spans.last_mut()
            && last.strong == next.strong
            && last.emphasis == next.emphasis
            && last.strikethrough == next.strikethrough
            && last.code == next.code
            && last.inline_math == next.inline_math
            && last.link == next.link
            && last.image.is_none()
            && next.image.is_none()
        {
            last.text.push_str(&next.text);
        } else {
            self.spans.push(next);
        }
    }

    fn flush_text(&mut self) {
        if self.in_table_cell || self.spans.is_empty() {
            return;
        }
        let pending = self
            .pending_kind
            .take()
            .unwrap_or(PendingTextKind::Paragraph);
        let alert = self.alerts.iter().rev().find_map(|alert| *alert);
        let kind = if let Some(kind) = alert {
            MarkdownBlockKind::Alert {
                kind,
                opens: std::mem::take(&mut self.alert_untitled),
            }
        } else if self.quote_depth > 0 {
            MarkdownBlockKind::Quote
        } else {
            match pending {
                PendingTextKind::Paragraph => MarkdownBlockKind::Paragraph,
                PendingTextKind::Heading(level) => MarkdownBlockKind::Heading(level),
                PendingTextKind::ListItem { depth } => MarkdownBlockKind::ListItem { depth },
            }
        };
        self.blocks.push(MarkdownBlock::Text {
            kind,
            spans: std::mem::take(&mut self.spans),
        });
    }

    fn finish_code_block(&mut self) {
        let Some((language, source)) = self.code_block.take() else {
            return;
        };
        let source = source.trim_end_matches(['\r', '\n']).to_owned();
        if language
            .as_deref()
            .is_some_and(|value| matches!(value.to_ascii_lowercase().as_str(), "mermaid" | "mmd"))
        {
            self.blocks.push(MarkdownBlock::Mermaid(source));
        } else if language.as_deref().is_some_and(|value| {
            matches!(
                value.to_ascii_lowercase().as_str(),
                "math" | "latex" | "tex"
            )
        }) {
            self.blocks.push(MarkdownBlock::DisplayMath(source));
        } else {
            self.blocks.push(MarkdownBlock::Code { language, source });
        }
    }

    fn finish_table_row(&mut self) {
        if self.table_row.is_empty() {
            return;
        }
        let Some(table) = self.table.as_mut() else {
            self.table_row.clear();
            return;
        };
        let row = normalize_table_row(std::mem::take(&mut self.table_row), table.alignments.len());
        if self.table_header && table.header.is_none() {
            table.header = Some(row);
        } else {
            table.rows.push(row);
        }
    }

    fn finish_table(&mut self) {
        let Some(table) = self.table.take() else {
            return;
        };
        let header = table.header.unwrap_or_default();
        if header.is_empty() && table.rows.is_empty() {
            return;
        }
        self.blocks.push(MarkdownBlock::Table(MarkdownTable {
            alignments: table.alignments,
            header,
            rows: table.rows,
        }));
    }

    fn finish(mut self) -> NativeMarkdown {
        self.flush_text();
        NativeMarkdown::from_blocks(self.blocks)
    }
}

fn heading_level(level: HeadingLevel) -> u8 {
    match level {
        HeadingLevel::H1 => 1,
        HeadingLevel::H2 => 2,
        HeadingLevel::H3 => 3,
        HeadingLevel::H4 => 4,
        HeadingLevel::H5 => 5,
        HeadingLevel::H6 => 6,
    }
}

fn table_alignment(alignment: CmarkAlignment) -> MarkdownTableAlignment {
    match alignment {
        CmarkAlignment::None | CmarkAlignment::Left => MarkdownTableAlignment::Left,
        CmarkAlignment::Center => MarkdownTableAlignment::Center,
        CmarkAlignment::Right => MarkdownTableAlignment::Right,
    }
}

fn normalize_table_row(
    mut row: Vec<Vec<MarkdownSpan>>,
    column_count: usize,
) -> Vec<Vec<MarkdownSpan>> {
    row.truncate(column_count);
    row.resize_with(column_count, Vec::new);
    row
}

fn normalize_math_delimiters(source: &str) -> String {
    let mut output = String::with_capacity(source.len());
    let mut fence: Option<char> = None;
    for line in source.split_inclusive('\n') {
        let trimmed = line.trim_start();
        if trimmed.starts_with("```") || trimmed.starts_with("~~~") {
            let marker = trimmed.chars().next().unwrap();
            if fence == Some(marker) {
                fence = None
            } else if fence.is_none() {
                fence = Some(marker)
            }
            output.push_str(line);
            continue;
        }
        if fence.is_some() {
            output.push_str(line);
            continue;
        }
        let mut chars = line.chars().peekable();
        let mut code = false;
        while let Some(c) = chars.next() {
            if c == '`' {
                code = !code;
                output.push(c);
                continue;
            }
            if !code && c == '\\' {
                match chars.peek().copied() {
                    Some('(') | Some(')') => {
                        chars.next();
                        output.push('$');
                        continue;
                    }
                    Some('[') | Some(']') => {
                        chars.next();
                        output.push_str("$$");
                        continue;
                    }
                    Some('\\') => {
                        output.push(c);
                        output.push(chars.next().unwrap());
                        continue;
                    }
                    _ => {}
                }
            }
            output.push(c)
        }
    }
    output
}

impl ComponentView for NativeMarkdown {
    const TYPE_ID: Option<&'static str> =
        Some(crate::component_descriptors::NATIVE_MARKDOWN.type_id);

    const BEHAVIOR: crate::TypeBehavior<Self> = crate::TypeBehavior {
        slot_assembler: Some(crate::AppContext::assemble_markdown),
        ..crate::TypeBehavior::NONE
    };

    fn share_layouts(
        &mut self,
        share: &mut dyn FnMut(&mut std::sync::Arc<nana_ui_core::LayoutStyle>),
    ) {
        share(&mut self.style.layout);
    }

    fn node_kind(&self) -> NodeKind {
        NodeKind::Element {
            tag: "markdown".into(),
        }
    }

    /// The selection lives in the shared [`TextSelectionGroup`].
    const ALWAYS_REPROJECT: bool = true;

    fn project(&self, id: StableNodeId, world: &UiWorld, mutations: &mut MutationQueue) {
        let plain = self.plain_text();
        if world.text(id) != Some(plain.as_str()) {
            mutations.set_text(
                id,
                TextContent {
                    value: plain.clone().into(),
                },
            );
        }
        let visual = NativeMarkdownVisual {
            blocks: self.blocks.clone().into(),
            text: Arc::from(plain),
            selection: projected_selection(&self.selection),
            code_presenter: Arc::clone(&self.code_presenter),
        };
        if world.widget_visual_as::<NativeMarkdownVisual>(id) != Some(&visual) {
            mutations.set_widget_visual(id, Some(visual));
        }
        let mut style = self.style.clone();
        let layout = Arc::make_mut(&mut style.layout);
        layout.width.get_or_insert(LengthSpec::Fill);
        layout.height = None;
        layout.min_height = Some(LengthSpec::Px(LINE_HEIGHT));
        project_common(
            id,
            world,
            mutations,
            &style,
            InteractionState {
                pointer_events: true,
                focusable: true,
            },
            AccessibilityState {
                role: AccessibilityRole::Document,
                value: self
                    .selected_text()
                    .map(Arc::from)
                    .or_else(|| Some(Arc::from(self.plain_text().as_str()))),
                ..AccessibilityState::default()
            },
        );
    }
}

/// Hidden fence identity slot. Assembly allocates these; `project` never invents IDs.
#[derive(Clone, Debug, PartialEq)]
struct MarkdownFenceChild {
    source: String,
    highlight: Option<HighlightRequest>,
    /// A diagram's title, description and counts, for its accessible name.
    diagram: Option<crate::markdown_diagram::Description>,
}

impl ComponentView for MarkdownFenceChild {
    fn node_kind(&self) -> NodeKind {
        NodeKind::Text
    }

    fn project(&self, id: StableNodeId, world: &UiWorld, mutations: &mut MutationQueue) {
        if world.text(id) != Some(self.source.as_str()) {
            mutations.set_text(
                id,
                TextContent {
                    value: self.source.clone().into(),
                },
            );
        }
        if world.highlight_request(id) != self.highlight.as_ref() {
            mutations.set_highlight_request(id, self.highlight.clone());
        }
        if world.widget_visual_ref(id).is_some() {
            mutations.set_widget_visual_handle(id, None);
        }
        let mut style = NodeStyle::default();
        Arc::make_mut(&mut style.layout).hidden = true;
        project_common(
            id,
            world,
            mutations,
            &style,
            InteractionState {
                pointer_events: false,
                focusable: false,
            },
            self.accessibility(world),
        );
    }
}

impl MarkdownFenceChild {
    /// The source as the value; a diagram also names itself (its title, or
    /// "diagram") and describes itself (its description, or its counts).
    fn accessibility(&self, world: &UiWorld) -> AccessibilityState {
        let mut state = AccessibilityState {
            role: AccessibilityRole::Text,
            value: Some(Arc::from(self.source.as_str())),
            ..AccessibilityState::default()
        };
        if let Some(diagram) = &self.diagram {
            let strings = world.framework_strings();
            state.label = Some(
                diagram
                    .title
                    .as_deref()
                    .map_or_else(|| Arc::clone(&strings.diagram_label), Arc::from),
            );
            state.description = diagram.description.as_deref().map(Arc::from).or_else(|| {
                (diagram.summary.nodes > 0).then(|| {
                    Arc::from(nana_ui_core::framework_strings::fill(
                        &strings.diagram_summary,
                        &[
                            ("nodes", &diagram.summary.nodes),
                            ("edges", &diagram.summary.edges),
                        ],
                    ))
                })
            });
        }
        state
    }
}

impl AppContext {
    /// Allocate one hidden text child per mermaid, display-math, and code fence.
    ///
    /// Child text is the fence source. Code uses [`HighlightRequest::highlight`];
    /// mermaid/math use presenters `"mermaid"` / `"math"` with the source as
    /// `language`. Children stay identity slots (`layout.hidden`) and do not
    /// emit Scene text or quads. Extra fence children are despawned. The
    /// parent is re-projected after the children are attached.
    pub fn assemble_markdown(
        &mut self,
        markdown: Entity<NativeMarkdown>,
    ) -> Result<bool, FrameworkError> {
        let parent = markdown.stable_id();
        let document = markdown_document(self, parent)?;
        let (slots, stored) = self.read(markdown, |markdown| {
            (
                markdown
                    .blocks
                    .iter()
                    .filter_map(|block| {
                        let mut slot = block.fence_child()?;
                        if matches!(block, MarkdownBlock::Code { .. })
                            && let Some(request) = slot.highlight.as_mut()
                        {
                            request.presenter = Arc::clone(&markdown.code_presenter);
                        }
                        Some(slot)
                    })
                    .collect::<Vec<_>>(),
                markdown.fence_children.clone(),
            )
        })?;
        let mut existing = stored
            .into_iter()
            .filter(|id| self.world().contains(*id))
            .collect::<Vec<_>>();
        if existing.is_empty() {
            existing = self
                .world()
                .node(parent)
                .map(|node| node.children)
                .unwrap_or_default();
        }

        let mut next = Vec::with_capacity(slots.len());
        let mut used = HashSet::new();
        for (index, slot) in slots.into_iter().enumerate() {
            if let Some(entity) = existing
                .get(index)
                .copied()
                .and_then(|id| fence_child_entity(self, id))
            {
                let id = entity.stable_id();
                self.update_component(entity, |child, _| {
                    *child = slot;
                })?;
                next.push(id);
                used.insert(id);
            } else {
                let entity = self.create_detached_component(document, slot)?;
                let id = entity.stable_id();
                next.push(id);
                used.insert(id);
            }
        }
        for id in existing {
            if !used.contains(&id) {
                drop_fence_child(self, id)?;
            }
        }

        reconcile_markdown_children(self, markdown, &next)?;
        self.update_component(markdown, |markdown, _| {
            markdown.fence_children = next;
        })?;
        Ok(true)
    }
}

fn markdown_document(context: &AppContext, id: StableNodeId) -> Result<DocumentId, FrameworkError> {
    context
        .world()
        .node(id)
        .map(|node| node.document)
        .ok_or(FrameworkError::MissingView(id))
}

fn fence_child_entity(
    context: &AppContext,
    id: StableNodeId,
) -> Option<Entity<MarkdownFenceChild>> {
    let entity = Entity::from_stable_id(id);
    context.read(entity, |_| ()).ok()?;
    Some(entity)
}

fn drop_fence_child(context: &mut AppContext, id: StableNodeId) -> Result<(), FrameworkError> {
    if fence_child_entity(context, id).is_some() {
        context.remove_view(Entity::<MarkdownFenceChild>::from_stable_id(id))?;
        return Ok(());
    }
    if context.world().contains(id) {
        let mut mutations = MutationQueue::new();
        mutations.despawn_subtree(id);
        context.commit_mutations(mutations)?;
    }
    Ok(())
}

fn reconcile_markdown_children(
    context: &mut AppContext,
    parent: Entity<NativeMarkdown>,
    ordered: &[StableNodeId],
) -> Result<bool, FrameworkError> {
    let parent_id = parent.stable_id();
    let current = context
        .world()
        .node(parent_id)
        .ok_or(FrameworkError::MissingView(parent_id))?
        .children
        .clone();
    if current.as_slice() == ordered {
        return Ok(false);
    }
    let keep = ordered.iter().copied().collect::<HashSet<_>>();
    for child in &current {
        if !keep.contains(child) {
            drop_fence_child(context, *child)?;
        }
    }
    let mut mutations = MutationQueue::new();
    for child in ordered {
        mutations.insert(parent_id, *child, None);
    }
    context.commit_mutations(mutations)?;
    Ok(true)
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RichSpan {
    pub text: Arc<str>,
    pub strong: bool,
    pub emphasis: bool,
    pub code: bool,
    pub link: Option<Arc<str>>,
    /// Semantic foreground of this span, resolved against the installed
    /// palette. `None` paints it in the node's own text color.
    pub color: Option<SemanticColorRole>,
}

impl RichSpan {
    pub fn plain(text: impl Into<Arc<str>>) -> Self {
        Self {
            text: text.into(),
            strong: false,
            emphasis: false,
            code: false,
            link: None,
            color: None,
        }
    }

    pub fn link(text: impl Into<Arc<str>>, href: impl Into<Arc<str>>) -> Self {
        Self {
            text: text.into(),
            strong: false,
            emphasis: false,
            code: false,
            link: Some(href.into()),
            color: None,
        }
    }

    /// Paint this span in `role` instead of the node's text color. Business
    /// colors do not belong here; pick the role that matches the span's job.
    pub fn color(mut self, role: SemanticColorRole) -> Self {
        self.color = Some(role);
        self
    }
}

/// Half-open grapheme range `[start, end)` in the laid-out document run.
///
/// Offsets count Unicode grapheme clusters, not UTF-8 bytes. Markdown block
/// separators (`\n\n`, `\n`, `\t`) appear in [`Self::text`] but are not
/// graphemes and do not advance `start`/`end`. [`NativeMarkdownVisual`]
/// and [`SelectableRichTextVisual`] project this same range.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TextSelectionSnapshot {
    pub start: usize,
    pub end: usize,
    pub text: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct TextSelectionGroupId(u64);

impl TextSelectionGroupId {
    fn next() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(1);
        Self(NEXT.fetch_add(1, Ordering::Relaxed))
    }
}

/// Shared document-level selection for one rich-text or markdown leaf.
#[derive(Clone)]
pub struct TextSelectionGroup {
    id: TextSelectionGroupId,
    state: Arc<Mutex<GroupState>>,
}

impl std::fmt::Debug for TextSelectionGroup {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("TextSelectionGroup")
            .field("id", &self.id)
            .finish()
    }
}

impl Default for TextSelectionGroup {
    fn default() -> Self {
        Self::new()
    }
}

impl TextSelectionGroup {
    pub fn new() -> Self {
        Self {
            id: TextSelectionGroupId::next(),
            state: Arc::new(Mutex::new(GroupState::default())),
        }
    }

    pub fn id(&self) -> TextSelectionGroupId {
        self.id
    }

    pub fn clear(&self) {
        if let Ok(mut state) = self.state.lock() {
            *state = GroupState::default();
        }
    }

    pub fn snapshot(&self) -> Option<TextSelectionSnapshot> {
        self.state.lock().ok().and_then(|state| state.snapshot())
    }

    fn begin(&self, run: &DocumentRun, x: f32, y: f32) -> bool {
        let Ok(mut state) = self.state.lock() else {
            return false;
        };
        let caret = run.caret_at(x, y);
        state.anchor = Some(caret);
        state.focus = Some(caret);
        state.dragging = true;
        state.pressed_link = run.link_at(x, y);
        state.cached = Some(run.snapshot_range(caret, caret));
        true
    }

    fn drag(&self, run: &DocumentRun, x: f32, y: f32) -> bool {
        let Ok(mut state) = self.state.lock() else {
            return false;
        };
        if !state.dragging {
            return false;
        }
        let caret = run.caret_at(x, y);
        if state.focus == Some(caret) {
            return false;
        }
        state.focus = Some(caret);
        if state.anchor != state.focus {
            state.pressed_link = None;
        }
        state.cached = Some(run.snapshot_range(state.anchor.unwrap_or(caret), caret));
        true
    }

    fn finish(&self, run: &DocumentRun, x: f32, y: f32) -> Option<RichTextEvent> {
        let Ok(mut state) = self.state.lock() else {
            return None;
        };
        if !state.dragging {
            return None;
        }
        state.dragging = false;
        let caret = run.caret_at(x, y);
        if let Some(focus) = state.focus.as_mut() {
            *focus = caret;
        }
        let snapshot = run.snapshot_range(state.anchor.unwrap_or(caret), caret);
        state.cached = Some(snapshot.clone());
        if let Some(link) = state.pressed_link.take()
            && snapshot
                .as_ref()
                .is_none_or(|value| value.start == value.end)
            && run.link_at(x, y).as_ref() == Some(&link)
        {
            state.anchor = Some(caret);
            state.focus = Some(caret);
            state.cached = None;
            return Some(RichTextEvent::LinkActivated(link));
        }
        Some(RichTextEvent::SelectionChanged(snapshot))
    }
}

#[derive(Debug, Default)]
struct GroupState {
    anchor: Option<usize>,
    focus: Option<usize>,
    dragging: bool,
    pressed_link: Option<Arc<str>>,
    cached: Option<Option<TextSelectionSnapshot>>,
}

impl GroupState {
    fn snapshot(&self) -> Option<TextSelectionSnapshot> {
        self.cached.clone().flatten()
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RichTextEvent {
    SelectionChanged(Option<TextSelectionSnapshot>),
    LinkActivated(Arc<str>),
    ImageActivated(MarkdownImage),
    /// A click (press and release without selecting) on a native mermaid
    /// diagram. The hook for showing it larger: the application decides
    /// whether and how, for example a dialog holding
    /// `NativeMarkdown::from_blocks([MarkdownBlock::Mermaid(source)])`
    /// wide enough for its natural size.
    DiagramActivated(MarkdownDiagram),
}

#[derive(Clone, Debug)]
pub struct SelectableRichText {
    spans: Vec<RichSpan>,
    selection: TextSelectionGroup,
    pub style: NodeStyle,
}

/// See [`NativeMarkdown`]'s `PartialEq`: every projected field takes part.
impl PartialEq for SelectableRichText {
    fn eq(&self, other: &Self) -> bool {
        self.spans == other.spans
            && self.selection.id == other.selection.id
            && self.style == other.style
    }
}

impl SelectableRichText {
    pub fn new(spans: impl IntoIterator<Item = RichSpan>) -> Self {
        Self {
            spans: spans.into_iter().collect(),
            selection: TextSelectionGroup::new(),
            style: NodeStyle::default(),
        }
    }

    pub fn spans(&self) -> &[RichSpan] {
        &self.spans
    }

    pub fn selection_group(&self) -> &TextSelectionGroup {
        &self.selection
    }

    pub fn group_id(&self) -> TextSelectionGroupId {
        self.selection.id()
    }

    pub fn style(mut self, style: NodeStyle) -> Self {
        self.style = style;
        self
    }

    pub fn plain_text(&self) -> String {
        self.spans.iter().map(|span| span.text.as_ref()).collect()
    }

    pub fn layout(&self, bounds: LayoutBox) -> RichTextGeometry {
        RichTextGeometry::of(layout_rich_spans(&self.spans, bounds))
    }

    pub fn pointer_down(&self, x: f32, y: f32, bounds: LayoutBox) -> bool {
        let geometry = self.layout(bounds);
        self.pointer_down_with_geometry(x, y, &geometry)
    }

    pub(crate) fn pointer_down_with_geometry(
        &self,
        x: f32,
        y: f32,
        geometry: &RichTextGeometry,
    ) -> bool {
        self.selection.begin(&geometry.run, x, y)
    }

    pub fn pointer_move(&self, x: f32, y: f32, bounds: LayoutBox) -> bool {
        let geometry = self.layout(bounds);
        self.pointer_move_with_geometry(x, y, &geometry)
    }

    pub(crate) fn pointer_move_with_geometry(
        &self,
        x: f32,
        y: f32,
        geometry: &RichTextGeometry,
    ) -> bool {
        self.selection.drag(&geometry.run, x, y)
    }

    pub fn pointer_up(&self, x: f32, y: f32, bounds: LayoutBox) -> Option<RichTextEvent> {
        let geometry = self.layout(bounds);
        self.pointer_up_with_geometry(x, y, &geometry)
    }

    pub(crate) fn pointer_up_with_geometry(
        &self,
        x: f32,
        y: f32,
        geometry: &RichTextGeometry,
    ) -> Option<RichTextEvent> {
        self.selection.finish(&geometry.run, x, y)
    }

    pub fn link_at(&self, x: f32, y: f32, bounds: LayoutBox) -> Option<Arc<str>> {
        self.layout(bounds).run.link_at(x, y)
    }

    pub fn selected_text(&self) -> Option<String> {
        self.selection.snapshot().map(|snapshot| snapshot.text)
    }

    pub fn selection_snapshot(&self) -> Option<TextSelectionSnapshot> {
        self.selection.snapshot()
    }

    pub fn copy_snapshot(&self) -> Option<TextSelectionSnapshot> {
        self.selection.snapshot()
    }

    pub fn clear_selection(&self) {
        self.selection.clear();
    }
}

impl ComponentView for SelectableRichText {
    fn share_layouts(
        &mut self,
        share: &mut dyn FnMut(&mut std::sync::Arc<nana_ui_core::LayoutStyle>),
    ) {
        share(&mut self.style.layout);
    }

    fn node_kind(&self) -> NodeKind {
        NodeKind::Element {
            tag: "rich-text".into(),
        }
    }

    /// The selection lives in the shared [`TextSelectionGroup`].
    const ALWAYS_REPROJECT: bool = true;

    fn project(&self, id: StableNodeId, world: &UiWorld, mutations: &mut MutationQueue) {
        let plain = self.plain_text();
        if world.text(id) != Some(plain.as_str()) {
            mutations.set_text(
                id,
                TextContent {
                    value: plain.clone().into(),
                },
            );
        }
        let empty = plain.is_empty();
        let visual = SelectableRichTextVisual {
            text: Arc::from(plain),
            selection: projected_selection(&self.selection),
        };
        if world.widget_visual_as::<SelectableRichTextVisual>(id) != Some(&visual) {
            mutations.set_widget_visual(id, Some(visual));
        }
        let colors = span_color::color_request(&self.spans);
        if world.highlight_request(id) != colors.as_ref() {
            mutations.set_highlight_request(id, colors);
        }
        let mut style = self.style.clone();
        let layout = Arc::make_mut(&mut style.layout);
        layout.width = Some(LengthSpec::Fill);
        // The box is as tall as the plain text measures at its width, wrapped
        // as the scene paints it. Only empty text, which measures no line, is
        // held open at one.
        layout.height = None;
        if empty {
            layout.min_height = Some(LengthSpec::Px(LINE_HEIGHT));
        }
        project_common(
            id,
            world,
            mutations,
            &style,
            InteractionState {
                pointer_events: true,
                focusable: true,
            },
            AccessibilityState {
                role: AccessibilityRole::Text,
                value: self
                    .selected_text()
                    .map(Arc::from)
                    .or_else(|| Some(Arc::from(self.plain_text().as_str()))),
                ..AccessibilityState::default()
            },
        );
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct GraphemeGeometry {
    pub index: usize,
    pub bounds: LayoutBox,
    pub grapheme: Arc<str>,
    pub span_index: usize,
    pub block_index: usize,
    pub link: Option<Arc<str>>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct RichTextGeometry {
    pub bounds: LayoutBox,
    pub graphemes: Vec<GraphemeGeometry>,
    run: DocumentRun,
}

impl RichTextGeometry {
    fn of(run: DocumentRun) -> Self {
        Self {
            bounds: run.bounds,
            graphemes: run.graphemes.clone(),
            run,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct MarkdownBlockGeometry {
    pub index: usize,
    pub bounds: LayoutBox,
    pub language: Option<Arc<str>>,
    pub label: Option<Arc<str>>,
    pub graphemes: Vec<GraphemeGeometry>,
    /// What draws the block when it is not text, computed once per layout.
    pub(crate) art: BlockArt,
}

/// A block drawn as a picture rather than as its text.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) enum BlockArt {
    #[default]
    None,
    /// A native diagram.
    Diagram(DiagramArt),
    /// A display formula.
    Drawing(Arc<crate::markdown_drawing::MarkdownDrawing>),
    /// Unreadable diagram source, shown as a code block.
    Code,
}

/// A laid-out diagram; equal when it is the same layout.
#[derive(Clone, Debug)]
pub(crate) struct DiagramArt(pub Arc<nana_ui_diagram::DiagramScene>);

impl PartialEq for DiagramArt {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}

impl BlockArt {
    /// Natural width of the picture, when there is one.
    pub(crate) fn width(&self) -> Option<f32> {
        match self {
            Self::Diagram(DiagramArt(scene)) => Some(scene.size[0]),
            Self::Drawing(drawing) => Some(drawing.width),
            Self::None | Self::Code => None,
        }
    }

    /// Height when placed into `width`, never scaled up.
    pub(crate) fn placed_height(&self, width: f32) -> Option<f32> {
        let (w, h) = match self {
            Self::Diagram(DiagramArt(scene)) => (scene.size[0], scene.size[1]),
            Self::Drawing(drawing) => (drawing.width, drawing.height),
            Self::None | Self::Code => return None,
        };
        Some(h * (width.max(1.0) / w.max(1.0)).min(1.0))
    }
}

impl MarkdownBlockGeometry {
    /// Where a native diagram shows inside this block, for a mermaid fence
    /// the diagram engine read; `None` for any other block.
    pub fn diagram_placement(&self) -> Option<MarkdownDiagramPlacement> {
        let BlockArt::Diagram(DiagramArt(scene)) = &self.art else {
            return None;
        };
        let natural = scene.size;
        let scale = (self.bounds.width.max(1.0) / natural[0].max(1.0)).min(1.0);
        let width = natural[0] * scale;
        Some(MarkdownDiagramPlacement {
            block_index: self.index,
            bounds: LayoutBox {
                x: self.bounds.x + (self.bounds.width - width).max(0.0) * 0.5,
                y: self.bounds.y,
                width,
                height: natural[1] * scale,
            },
            natural_size: natural,
            scale,
        })
    }
}

/// How a mermaid diagram sits in its [`NativeMarkdown`]: the box it is
/// drawn in (layout coordinates, the same space as the node's layout box),
/// the size it was laid out at, and the scale between them. Diagrams wider
/// than the column shrink to fit (`scale < 1`) and never grow; a preview
/// that shows one at `natural_size` reads it at full size.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MarkdownDiagramPlacement {
    pub block_index: usize,
    pub bounds: LayoutBox,
    pub natural_size: [f32; 2],
    pub scale: f32,
}

/// A mermaid diagram a click landed on ([`RichTextEvent::DiagramActivated`]).
/// `source` is the fence body; [`UiWorld::markdown_diagram_placement`] says
/// where it shows and how far it is scaled down.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MarkdownDiagram {
    pub block_index: usize,
    pub source: Arc<str>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct MarkdownGeometry {
    pub bounds: LayoutBox,
    pub blocks: Vec<MarkdownBlockGeometry>,
    run: DocumentRun,
}

#[derive(Clone, Debug, PartialEq)]
struct DocumentRun {
    bounds: LayoutBox,
    graphemes: Vec<GraphemeGeometry>,
    separators: Vec<&'static str>,
}

impl DocumentRun {
    fn empty(bounds: LayoutBox) -> Self {
        Self {
            bounds,
            graphemes: Vec::new(),
            separators: Vec::new(),
        }
    }

    fn caret_at(&self, x: f32, y: f32) -> usize {
        let length = self.graphemes.len();
        if length == 0 {
            return 0;
        }
        let first_top = self
            .graphemes
            .iter()
            .map(|item| item.bounds.y)
            .min_by(f32::total_cmp)
            .unwrap_or(self.bounds.y);
        let last_bottom = self
            .graphemes
            .iter()
            .map(|item| item.bounds.y + item.bounds.height)
            .max_by(f32::total_cmp)
            .unwrap_or(self.bounds.y + self.bounds.height);
        if y < first_top {
            return 0;
        }
        if y > last_bottom {
            return length;
        }
        // The line comes first: the nearest grapheme vertically, and only
        // then the nearest along that line. A straight-line distance would
        // let the end of a long line above beat the end of a short last line
        // for a point to the right of both.
        let Some((index, bounds)) = self
            .graphemes
            .iter()
            .map(|item| (item.index, item.bounds))
            .min_by(|(_, left), (_, right)| {
                let (left_x, left_y) = point_box_distance(x, y, *left);
                let (right_x, right_y) = point_box_distance(x, y, *right);
                left_y.total_cmp(&right_y).then(left_x.total_cmp(&right_x))
            })
        else {
            return 0;
        };
        if x >= bounds.x + bounds.width / 2.0 {
            index.saturating_add(1).min(length)
        } else {
            index
        }
    }

    fn link_at(&self, x: f32, y: f32) -> Option<Arc<str>> {
        self.graphemes
            .iter()
            .find(|item| item.bounds.contains(x, y))
            .and_then(|item| item.link.clone())
    }

    fn snapshot_range(&self, anchor: usize, focus: usize) -> Option<TextSelectionSnapshot> {
        let length = self.graphemes.len();
        let start = anchor.min(focus).min(length);
        let end = anchor.max(focus).min(length);
        if start == end {
            return None;
        }
        let mut text = String::new();
        for index in start..end {
            if !text.is_empty() {
                text.push_str(self.separators.get(index).copied().unwrap_or(""));
            }
            if let Some(item) = self.graphemes.get(index) {
                text.push_str(item.grapheme.as_ref());
            }
        }
        Some(TextSelectionSnapshot { start, end, text })
    }
}

/// The advance of every grapheme of `text` (`graphemes(true)` order) when the
/// whole of it is shaped as one unwrapped line at `size` / `weight` /
/// italic. One call
/// per line of a span rather than one per grapheme: kerning, ligatures and
/// joining scripts only come out right when the run is shaped together, which
/// is how the painter draws it.
pub(crate) type MarkdownMeasure<'a> = dyn Fn(&str, f32, u16, bool) -> Vec<f32> + 'a;

struct LayoutCursor<'a> {
    measure: Option<&'a MarkdownMeasure<'a>>,
    font_size: Option<f32>,
    font_weight: u16,
    font_italic: bool,
    origin_x: f32,
    max_width: f32,
    line_height: f32,
    // Inline advance stays local: accumulating viewport coordinates changes
    // rounding and can wrap a final glyph that fitted during origin-zero measure.
    x: f32,
    y: f32,
    line_start_x: f32,
}

impl<'a> LayoutCursor<'a> {
    fn new(origin_x: f32, y: f32, max_width: f32, line_height: f32) -> Self {
        Self {
            measure: None,
            font_size: None,
            font_weight: nana_ui_core::TypographyTokens::DEFAULT.regular,
            font_italic: false,
            origin_x,
            max_width,
            line_height,
            x: 0.0,
            y,
            line_start_x: 0.0,
        }
    }

    fn markdown(
        mut self,
        measure: Option<&'a MarkdownMeasure<'a>>,
        size: f32,
        weight: u16,
    ) -> Self {
        self.measure = measure;
        self.font_size = Some(size);
        self.font_weight = weight;
        self
    }

    /// Place a run of graphemes that contains no newline, measured together.
    fn place_segment(&mut self, graphemes: &[&str]) -> Vec<LayoutBox> {
        let advances = match (self.measure, self.font_size) {
            (Some(measure), Some(size)) => {
                let advances = measure(
                    &graphemes.concat(),
                    size,
                    self.font_weight,
                    self.font_italic,
                );
                (advances.len() == graphemes.len()).then_some(advances)
            }
            _ => None,
        };
        graphemes
            .iter()
            .enumerate()
            .map(|(index, grapheme)| match &advances {
                Some(advances) => self.advance_by(advances[index]),
                None => self.place(grapheme),
            })
            .collect()
    }

    /// Place every grapheme of `text`, measuring each newline-free stretch as
    /// one run.
    fn place_text(&mut self, graphemes: &[&str]) -> Vec<LayoutBox> {
        let mut bounds = Vec::with_capacity(graphemes.len());
        for segment in graphemes.split_inclusive(|grapheme| is_newline(grapheme)) {
            let (body, newline) = match segment.split_last() {
                Some((last, body)) if is_newline(last) => (body, Some(*last)),
                _ => (segment, None),
            };
            bounds.extend(self.place_segment(body));
            if let Some(newline) = newline {
                bounds.push(self.place(newline));
            }
        }
        bounds
    }

    fn place(&mut self, grapheme: &str) -> LayoutBox {
        if is_newline(grapheme) {
            let bounds = LayoutBox {
                x: self.origin_x + self.x,
                y: self.y,
                width: 0.0,
                height: self.line_height,
            };
            self.x = self.line_start_x;
            self.y += self.line_height;
            return bounds;
        }
        // Unmeasured: measured graphemes go through `place_segment`.
        let advance = self.font_size.map_or(GRAPHEME_ADVANCE, |size| {
            crate::markdown_drawing::text_advance(grapheme, size)
        });
        self.advance_by(advance)
    }

    fn advance_by(&mut self, advance: f32) -> LayoutBox {
        if self.max_width.is_finite()
            && self.x > self.line_start_x
            && self.x + advance > self.max_width
        {
            self.x = self.line_start_x;
            self.y += self.line_height;
        }
        let bounds = LayoutBox {
            x: self.origin_x + self.x,
            y: self.y,
            width: advance,
            height: self.line_height,
        };
        self.x += advance;
        bounds
    }

    fn height_from(&self, top: f32) -> f32 {
        (self.y + self.line_height - top).max(self.line_height)
    }
}

fn layout_rich_spans(spans: &[RichSpan], bounds: LayoutBox) -> DocumentRun {
    let mut graphemes = Vec::new();
    let mut separators = Vec::new();
    let mut cursor = LayoutCursor::new(bounds.x, bounds.y, usable_width(bounds.width), LINE_HEIGHT);
    for (span_index, span) in spans.iter().enumerate() {
        for grapheme in span.text.graphemes(true) {
            if grapheme.is_empty() {
                continue;
            }
            let index = graphemes.len();
            separators.push("");
            graphemes.push(GraphemeGeometry {
                index,
                bounds: cursor.place(grapheme),
                grapheme: Arc::from(grapheme),
                span_index,
                block_index: 0,
                link: span.link.clone(),
            });
        }
    }
    let height = if graphemes.is_empty() {
        LINE_HEIGHT
    } else {
        cursor.height_from(bounds.y)
    };
    DocumentRun {
        bounds: LayoutBox { height, ..bounds },
        graphemes,
        separators,
    }
}

/// Rich-span geometry read off the `nana-text` layout of the spans'
/// concatenated text, placed at the content box `bounds`.
///
/// A grapheme the layout draws nothing for (collapsed whitespace) gets a
/// zero-width box where the previous one ended, so offsets stay contiguous.
pub(crate) fn layout_rich_spans_shaped(
    spans: &[RichSpan],
    bounds: LayoutBox,
    layout: &nana_text::TextLayout,
) -> RichTextGeometry {
    let mut graphemes = Vec::new();
    let mut offset = 0;
    let mut previous = LayoutBox {
        width: 0.0,
        height: layout
            .lines
            .first()
            .map_or(LINE_HEIGHT, |line| line.bounds.height),
        ..bounds
    };
    for (span_index, span) in spans.iter().enumerate() {
        for (start, grapheme) in span.text.grapheme_indices(true) {
            let start = offset + start;
            previous = layout
                .selection_rects(start..start + grapheme.len())
                .into_iter()
                .reduce(nana_text::TextRect::union)
                .map_or(
                    LayoutBox {
                        x: previous.x + previous.width,
                        width: 0.0,
                        ..previous
                    },
                    |rect| text_rect_at(bounds, rect),
                );
            graphemes.push(GraphemeGeometry {
                index: graphemes.len(),
                bounds: previous,
                grapheme: Arc::from(grapheme),
                span_index,
                block_index: 0,
                link: span.link.clone(),
            });
        }
        offset += span.text.len();
    }
    let (_, height) = layout.physical_size();
    RichTextGeometry::of(DocumentRun {
        bounds: LayoutBox {
            height: height.max(LINE_HEIGHT),
            ..bounds
        },
        separators: vec![""; graphemes.len()],
        graphemes,
    })
}

/// A `nana-text` rect, which is relative to the text's origin, on the page of
/// the content box `origin`.
pub(crate) fn text_rect_at(origin: LayoutBox, rect: nana_text::TextRect) -> LayoutBox {
    LayoutBox {
        x: origin.x + rect.x,
        y: origin.y + rect.y,
        width: rect.width,
        height: rect.height,
    }
}

pub(crate) fn layout_markdown(blocks: &[MarkdownBlock], bounds: LayoutBox) -> MarkdownGeometry {
    layout_markdown_measured(blocks, bounds, None, 0)
}

/// `measure_key` names the fonts and shaping behind `measure`: diagrams
/// cache their layout under it.
pub(crate) fn layout_markdown_measured(
    blocks: &[MarkdownBlock],
    bounds: LayoutBox,
    measure: Option<&MarkdownMeasure<'_>>,
    measure_key: u64,
) -> MarkdownGeometry {
    let mut run = DocumentRun::empty(bounds);
    let mut block_geometry = Vec::with_capacity(blocks.len());
    let mut y = bounds.y;
    for (index, block) in blocks.iter().enumerate() {
        if index > 0 {
            y += BLOCK_GAP;
        }
        let geometry = layout_block(
            index,
            block,
            bounds.x,
            y,
            bounds.width,
            &mut run,
            measure,
            measure_key,
        );
        y = geometry.bounds.y + geometry.bounds.height;
        block_geometry.push(geometry);
    }
    let height = (y - bounds.y).max(if blocks.is_empty() { LINE_HEIGHT } else { 0.0 });
    run.bounds = LayoutBox { height, ..bounds };
    MarkdownGeometry {
        bounds: run.bounds,
        blocks: block_geometry,
        run,
    }
}

pub(crate) fn markdown_content_width(blocks: &[MarkdownBlock], geometry: &MarkdownGeometry) -> f32 {
    blocks
        .iter()
        .zip(&geometry.blocks)
        .map(|(_, geometry)| {
            if let Some(width) = geometry.art.width() {
                return width.min(geometry.bounds.width);
            }
            geometry
                .graphemes
                .iter()
                .map(|grapheme| grapheme.bounds.x + grapheme.bounds.width - geometry.bounds.x)
                .fold(0.0, f32::max)
        })
        .fold(0.0, f32::max)
}

#[allow(clippy::too_many_arguments)]
fn layout_block(
    index: usize,
    block: &MarkdownBlock,
    x: f32,
    y: f32,
    width: f32,
    run: &mut DocumentRun,
    measure: Option<&MarkdownMeasure<'_>>,
    measure_key: u64,
) -> MarkdownBlockGeometry {
    match block {
        MarkdownBlock::Text { kind, spans } => {
            let indent = text_indent(*kind);
            let line_height = text_line_height(*kind);
            // An alert's first paragraph sits under its title line.
            let title = match kind {
                MarkdownBlockKind::Alert { opens: true, .. } => ALERT_TITLE_HEIGHT,
                _ => 0.0,
            };
            let mut cursor = LayoutCursor::new(
                x + indent,
                y + title,
                usable_width((width - indent).max(0.0)),
                line_height,
            )
            .markdown(
                measure,
                crate::markdown_drawing::text_style(*kind).0,
                crate::markdown_drawing::text_style(*kind).1,
            );
            let start = run.graphemes.len();
            push_spans(
                run,
                &mut cursor,
                index,
                spans,
                if start == 0 { "" } else { "\n\n" },
            );
            let height = if spans.is_empty() && title > 0.0 {
                title
            } else {
                title + cursor.height_from(y + title)
            };
            labeled_block(
                index,
                x,
                y,
                width,
                height,
                None,
                None,
                run.graphemes[start..].to_vec(),
            )
        }
        MarkdownBlock::Code { language, source } => {
            // A fence is a panel: its text is inset from the block edge, and
            // the drawing paints the panel behind it.
            let inner_width = (usable_width(width) - CODE_PANEL_PADDING_X * 2.0).max(1.0);
            let mut cursor = LayoutCursor::new(
                x + CODE_PANEL_PADDING_X,
                y + CODE_PANEL_PADDING_Y,
                inner_width,
                LINE_HEIGHT,
            )
            .markdown(measure, 13.0, 400);
            let start = run.graphemes.len();
            push_source(
                run,
                &mut cursor,
                index,
                source,
                if start == 0 { "" } else { "\n\n" },
            );
            labeled_block(
                index,
                x,
                y,
                width,
                cursor.height_from(y + CODE_PANEL_PADDING_Y) + CODE_PANEL_PADDING_Y * 2.0,
                language
                    .as_deref()
                    .filter(|value| !value.is_empty())
                    .map(Arc::from),
                None,
                run.graphemes[start..].to_vec(),
            )
        }
        MarkdownBlock::DisplayMath(source) | MarkdownBlock::Mermaid(source) => {
            let art = match block {
                MarkdownBlock::Mermaid(_) => {
                    let measurer = crate::markdown_diagram::Measurer {
                        advances: measure,
                        key: measure_key,
                    };
                    match crate::markdown_diagram::layout(source, &measurer) {
                        crate::markdown_diagram::Outcome::Native(scene) => {
                            BlockArt::Diagram(DiagramArt(scene))
                        }
                        crate::markdown_diagram::Outcome::Fallback => BlockArt::Code,
                    }
                }
                _ => crate::markdown_drawing::block_drawing(block)
                    .map_or(BlockArt::None, |d| BlockArt::Drawing(Arc::new(d))),
            };
            if art == BlockArt::Code {
                // Source the engine cannot read reads as a code block.
                let fallback = MarkdownBlock::Code {
                    language: Some("mermaid".into()),
                    source: source.clone(),
                };
                let mut geometry =
                    layout_block(index, &fallback, x, y, width, run, measure, measure_key);
                geometry.art = BlockArt::Code;
                return geometry;
            }
            let mut cursor = LayoutCursor::new(x, y, usable_width(width), LINE_HEIGHT)
                .markdown(measure, 13.0, 400);
            let start = run.graphemes.len();
            push_source(
                run,
                &mut cursor,
                index,
                source,
                if start == 0 { "" } else { "\n\n" },
            );
            let height = art
                .placed_height(width)
                .unwrap_or_else(|| cursor.height_from(y));
            if art != BlockArt::None {
                let count = (run.graphemes.len() - start).max(1) as f32;
                for (offset, grapheme) in run.graphemes[start..].iter_mut().enumerate() {
                    grapheme.bounds = LayoutBox {
                        x: x + offset as f32 * width / count,
                        y,
                        width: width / count,
                        height,
                    };
                }
            }
            let mut geometry = labeled_block(
                index,
                x,
                y,
                width,
                height,
                None,
                None,
                run.graphemes[start..].to_vec(),
            );
            geometry.art = art;
            geometry
        }
        MarkdownBlock::Table(table) => layout_table(index, table, x, y, width, run, measure),
        MarkdownBlock::Rule => labeled_block(
            index,
            x,
            y,
            width,
            1.0,
            None,
            Some(Arc::from("---")),
            Vec::new(),
        ),
    }
}

fn layout_table(
    index: usize,
    table: &MarkdownTable,
    x: f32,
    y: f32,
    width: f32,
    run: &mut DocumentRun,
    measure: Option<&MarkdownMeasure<'_>>,
) -> MarkdownBlockGeometry {
    let start = run.graphemes.len();
    let mut cursor =
        LayoutCursor::new(x, y, usable_width(width), LINE_HEIGHT).markdown(measure, 13.0, 400);
    let mut first_cell = true;
    let mut span_offset = 0;
    for (row_index, row) in std::iter::once(table.header.as_slice())
        .chain(table.rows.iter().map(Vec::as_slice))
        .enumerate()
    {
        for (column_index, cell) in row.iter().enumerate() {
            let separator = if first_cell {
                if start == 0 { "" } else { "\n\n" }
            } else if column_index == 0 {
                "\n"
            } else {
                "\t"
            };
            if row_index > 0 && column_index == 0 {
                cursor.x = cursor.line_start_x;
                cursor.y += cursor.line_height;
            }
            let cell_start = run.graphemes.len();
            push_spans(run, &mut cursor, index, cell, separator);
            for grapheme in &mut run.graphemes[cell_start..] {
                grapheme.span_index += span_offset;
            }
            span_offset += cell.len();
            first_cell = false;
        }
    }
    labeled_block(
        index,
        x,
        y,
        width,
        cursor.height_from(y),
        None,
        None,
        run.graphemes[start..].to_vec(),
    )
}

#[allow(clippy::too_many_arguments)]
fn labeled_block(
    index: usize,
    x: f32,
    y: f32,
    width: f32,
    height: f32,
    language: Option<Arc<str>>,
    label: Option<Arc<str>>,
    graphemes: Vec<GraphemeGeometry>,
) -> MarkdownBlockGeometry {
    MarkdownBlockGeometry {
        index,
        bounds: LayoutBox {
            x,
            y,
            width,
            height,
        },
        language,
        label,
        graphemes,
        art: BlockArt::None,
    }
}

fn push_spans(
    run: &mut DocumentRun,
    cursor: &mut LayoutCursor,
    block_index: usize,
    spans: &[MarkdownSpan],
    first_separator: &'static str,
) {
    let mut first = true;
    let base_weight = cursor.font_weight;
    for (span_index, span) in spans.iter().enumerate() {
        cursor.font_weight = if span.strong { 700 } else { base_weight };
        cursor.font_italic = span.emphasis;
        if let Some(resource) = span
            .image_resource
            .as_ref()
            .filter(|resource| resource.width > 0 && resource.height > 0)
        {
            let scale = (cursor.max_width / resource.width as f32).clamp(0.0, 1.0);
            let width = resource.width as f32 * scale;
            let height = resource.height as f32 * scale;
            if cursor.x > cursor.line_start_x && cursor.x + width > cursor.max_width {
                cursor.x = cursor.line_start_x;
                cursor.y += cursor.line_height;
            }
            cursor.line_height = cursor.line_height.max(height);
            let mut graphemes = span.text.graphemes(true).collect::<Vec<_>>();
            if graphemes.is_empty() {
                graphemes.push("");
            }
            for (offset, grapheme) in graphemes.iter().enumerate() {
                run.separators
                    .push(if first { first_separator } else { "" });
                first = false;
                run.graphemes.push(GraphemeGeometry {
                    index: run.graphemes.len(),
                    bounds: LayoutBox {
                        x: cursor.origin_x
                            + cursor.x
                            + offset as f32 * width / graphemes.len() as f32,
                        y: cursor.y,
                        width: width / graphemes.len() as f32,
                        height,
                    },
                    grapheme: Arc::from(*grapheme),
                    span_index,
                    block_index,
                    link: span.image.as_deref().map(Arc::from),
                });
            }
            cursor.x += width;
            continue;
        }
        if span.inline_math {
            let drawing = crate::markdown_drawing::math(&span.text, 13.0);
            let scale = (cursor.max_width / drawing.width.max(1.0)).min(1.0);
            let width = drawing.width * scale;
            if cursor.x > cursor.line_start_x && cursor.x + width > cursor.max_width {
                cursor.x = cursor.line_start_x;
                cursor.y += cursor.line_height;
            }
            cursor.line_height = cursor.line_height.max(drawing.height * scale);
            let graphemes = span.text.graphemes(true).collect::<Vec<_>>();
            for (offset, grapheme) in graphemes.iter().enumerate() {
                run.separators
                    .push(if first { first_separator } else { "" });
                first = false;
                run.graphemes.push(GraphemeGeometry {
                    index: run.graphemes.len(),
                    bounds: LayoutBox {
                        x: cursor.origin_x
                            + cursor.x
                            + offset as f32 * width / graphemes.len().max(1) as f32,
                        y: cursor.y,
                        width: width / graphemes.len().max(1) as f32,
                        height: cursor.line_height,
                    },
                    grapheme: Arc::from(*grapheme),
                    span_index,
                    block_index,
                    link: None,
                });
            }
            cursor.x += width;
            continue;
        }
        let graphemes = span
            .text
            .graphemes(true)
            .filter(|grapheme| !grapheme.is_empty())
            .collect::<Vec<_>>();
        let link = span
            .image
            .as_deref()
            .or(span.link.as_deref())
            .map(Arc::<str>::from);
        let bounds = cursor.place_text(&graphemes);
        for (grapheme, bounds) in graphemes.into_iter().zip(bounds) {
            let separator = if first { first_separator } else { "" };
            first = false;
            push_placed(
                run,
                block_index,
                span_index,
                grapheme,
                bounds,
                link.clone(),
                separator,
            );
        }
    }
}

fn push_source(
    run: &mut DocumentRun,
    cursor: &mut LayoutCursor,
    block_index: usize,
    source: &str,
    first_separator: &'static str,
) {
    let graphemes = source
        .graphemes(true)
        .filter(|grapheme| !grapheme.is_empty())
        .collect::<Vec<_>>();
    let bounds = cursor.place_text(&graphemes);
    for (index, (grapheme, bounds)) in graphemes.into_iter().zip(bounds).enumerate() {
        let separator = if index == 0 { first_separator } else { "" };
        push_placed(run, block_index, 0, grapheme, bounds, None, separator);
    }
}

fn push_placed(
    run: &mut DocumentRun,
    block_index: usize,
    span_index: usize,
    grapheme: &str,
    bounds: LayoutBox,
    link: Option<Arc<str>>,
    separator: &'static str,
) {
    let index = run.graphemes.len();
    run.separators.push(separator);
    run.graphemes.push(GraphemeGeometry {
        index,
        bounds,
        grapheme: Arc::from(grapheme),
        span_index,
        block_index,
        link,
    });
}

fn projected_selection(group: &TextSelectionGroup) -> Option<(usize, usize)> {
    group
        .snapshot()
        .map(|snapshot| (snapshot.start, snapshot.end))
}

fn markdown_block_plain_text(block: &MarkdownBlock) -> String {
    match block {
        MarkdownBlock::Text { spans, .. } => markdown_spans_plain_text(spans),
        MarkdownBlock::Code { source, .. }
        | MarkdownBlock::DisplayMath(source)
        | MarkdownBlock::Mermaid(source) => source.clone(),
        MarkdownBlock::Table(table) => std::iter::once(table.header.as_slice())
            .chain(table.rows.iter().map(Vec::as_slice))
            .filter(|row| !row.is_empty())
            .map(|row| {
                row.iter()
                    .map(|cell| markdown_spans_plain_text(cell))
                    .collect::<Vec<_>>()
                    .join("\t")
            })
            .collect::<Vec<_>>()
            .join("\n"),
        MarkdownBlock::Rule => "---".to_owned(),
    }
}

fn markdown_spans_plain_text(spans: &[MarkdownSpan]) -> String {
    spans.iter().map(|span| span.text.as_str()).collect()
}

fn collect_markdown_images(spans: &[MarkdownSpan], images: &mut Vec<MarkdownImage>) {
    for span in spans {
        let Some(source) = &span.image else {
            continue;
        };
        let image = MarkdownImage {
            source: source.clone(),
            alt: span.text.clone(),
        };
        if !images.contains(&image) {
            images.push(image);
        }
    }
}

pub(crate) fn text_indent(kind: MarkdownBlockKind) -> f32 {
    match kind {
        MarkdownBlockKind::Quote | MarkdownBlockKind::Alert { .. } => QUOTE_INDENT,
        MarkdownBlockKind::ListItem { depth } => depth.saturating_sub(1) as f32 * LIST_INDENT,
        _ => 0.0,
    }
}

fn text_line_height(kind: MarkdownBlockKind) -> f32 {
    match kind {
        MarkdownBlockKind::Heading(level) => {
            (24.0 - f32::from(level.saturating_sub(1)) * 1.5).max(LINE_HEIGHT)
        }
        _ => LINE_HEIGHT,
    }
}

fn usable_width(width: f32) -> f32 {
    if width.is_finite() && width > 0.0 {
        width
    } else {
        f32::INFINITY
    }
}

fn is_newline(value: &str) -> bool {
    matches!(value, "\n" | "\r" | "\r\n" | "\n\r")
}

/// Distance from a point to a box along each axis, `(dx, dy)`.
fn point_box_distance(x: f32, y: f32, bounds: LayoutBox) -> (f32, f32) {
    let dx = if x < bounds.x {
        bounds.x - x
    } else if x > bounds.x + bounds.width {
        x - (bounds.x + bounds.width)
    } else {
        0.0
    };
    let dy = if y < bounds.y {
        bounds.y - y
    } else if y > bounds.y + bounds.height {
        y - (bounds.y + bounds.height)
    } else {
        0.0
    };
    (dx, dy)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::DocumentId;
    use crate::framework::AppContext;

    fn document() -> DocumentId {
        DocumentId::new(1).unwrap()
    }

    fn bounds(width: f32, height: f32) -> LayoutBox {
        LayoutBox {
            x: 0.0,
            y: 0.0,
            width,
            height,
        }
    }

    fn caret_x(index: usize) -> f32 {
        index as f32 * GRAPHEME_ADVANCE + 1.0
    }

    #[test]
    fn from_source_markdown_projects_heading_and_strong_plain_text() {
        let markdown = NativeMarkdown::from_source("# Title\n\nHello **world**");
        let plain = markdown.plain_text();
        assert!(plain.contains("Title"), "{plain}");
        assert!(plain.contains("Hello world"), "{plain}");
        assert!(matches!(
            markdown.blocks().first(),
            Some(MarkdownBlock::Text {
                kind: MarkdownBlockKind::Heading(1),
                ..
            })
        ));
        assert!(markdown.blocks().iter().any(|block| matches!(
            block,
            MarkdownBlock::Text {
                kind: MarkdownBlockKind::Paragraph,
                spans,
            } if spans.iter().any(|span| span.strong && span.text == "world")
        )));
        assert_eq!(NativeMarkdown::parse("# Title").plain_text(), "Title");
    }

    #[test]
    fn markdown_ten_thousand_deep_parses_and_lays_out_on_a_small_stack() {
        let deep = [
            format!("{} quoted", ">".repeat(10_000)),
            format!("{}item", "- ".repeat(10_000)),
            format!("{}x{}", "*".repeat(10_000), "*".repeat(10_000)),
            format!("{}x{}", "[".repeat(10_000), "](u)".repeat(10_000)),
        ];
        std::thread::Builder::new()
            .stack_size(512 * 1024)
            .spawn(move || {
                for source in deep {
                    let markdown = NativeMarkdown::parse(&source);
                    let geometry = layout_markdown(markdown.blocks(), bounds(400.0, 100.0));
                    assert_eq!(geometry.blocks.len(), markdown.blocks().len());
                }
            })
            .unwrap()
            .join()
            .unwrap();
        let shallow = NativeMarkdown::parse(">> quoted\n\n- - item");
        let kinds: Vec<_> = shallow
            .blocks()
            .iter()
            .filter_map(|block| match block {
                MarkdownBlock::Text { kind, .. } => Some(*kind),
                _ => None,
            })
            .collect();
        assert!(kinds.contains(&MarkdownBlockKind::Quote), "{kinds:?}");
        assert!(
            kinds.contains(&MarkdownBlockKind::ListItem { depth: 2 }),
            "{kinds:?}"
        );
    }

    #[test]
    fn markdown_line_breaks_are_invariant_under_viewport_translation() {
        let markdown = NativeMarkdown::parse("NATIVE_PARITY_FORK_A_20260903");
        let measure =
            |value: &str, _: f32, _: u16, _: bool| vec![7.2; value.graphemes(true).count()];
        let natural =
            layout_markdown_measured(markdown.blocks(), bounds(1000.0, 100.0), Some(&measure), 1);
        let width = markdown_content_width(markdown.blocks(), &natural);
        for x in [0.0, 681.0, 10000.0] {
            let placed = layout_markdown_measured(
                markdown.blocks(),
                LayoutBox {
                    x,
                    width,
                    ..bounds(width, 100.0)
                },
                Some(&measure),
                1,
            );
            assert_eq!(
                placed.bounds.height, natural.bounds.height,
                "moving a paragraph must not introduce another line at x={x}"
            );
            assert_eq!(placed.run.graphemes.len(), natural.run.graphemes.len());
        }
    }

    #[test]
    fn from_source_markdown_maps_list_quote_code_table_and_rule() {
        let markdown = NativeMarkdown::from_source(
            "> quoted\n\n- item\n\n```rust\nfn main() {}\n```\n\n| A | B |\n| --- | ---: |\n| **x** | `1` |\n\n---\n\nSee [docs](https://example.com) and ~~old~~.\n",
        );
        assert!(markdown.blocks().iter().any(|block| matches!(
            block,
            MarkdownBlock::Text {
                kind: MarkdownBlockKind::Quote,
                ..
            }
        )));
        assert!(markdown.blocks().iter().any(|block| matches!(
            block,
            MarkdownBlock::Text {
                kind: MarkdownBlockKind::ListItem { .. },
                ..
            }
        )));
        assert!(markdown.blocks().iter().any(|block| matches!(
            block,
            MarkdownBlock::Code {
                language: Some(language),
                source,
            } if language == "rust" && source.contains("fn main")
        )));
        assert!(markdown.blocks().iter().any(|block| matches!(
            block,
            MarkdownBlock::Table(table)
                if table.alignments.last() == Some(&MarkdownTableAlignment::Right)
                    && table.rows[0][0].iter().any(|span| span.strong)
                    && table.rows[0][1].iter().any(|span| span.code)
        )));
        assert!(
            markdown
                .blocks()
                .iter()
                .any(|block| matches!(block, MarkdownBlock::Rule))
        );
        assert!(markdown.blocks().iter().any(|block| matches!(
            block,
            MarkdownBlock::Text { spans, .. }
                if spans.iter().any(|span| span.link.as_deref() == Some("https://example.com"))
                    && spans.iter().any(|span| span.strikethrough)
        )));
    }

    #[test]
    fn heading_and_paragraph_blocks_are_distinct() {
        let markdown = NativeMarkdown::from_blocks([
            MarkdownBlock::heading(1, [MarkdownSpan::plain("Title")]),
            MarkdownBlock::paragraph([MarkdownSpan::plain("Body")]),
            MarkdownBlock::DisplayMath("\\frac{1}{2}".into()),
            MarkdownBlock::Mermaid("flowchart LR\nA-->B".into()),
        ]);
        assert!(matches!(
            markdown.blocks()[0],
            MarkdownBlock::Text {
                kind: MarkdownBlockKind::Heading(1),
                ..
            }
        ));
        assert!(matches!(
            markdown.blocks()[1],
            MarkdownBlock::Text {
                kind: MarkdownBlockKind::Paragraph,
                ..
            }
        ));
        assert_eq!(
            markdown.plain_text(),
            "Title\n\nBody\n\n\\frac{1}{2}\n\nflowchart LR\nA-->B"
        );
        let geometry = markdown.layout(bounds(400.0, 200.0));
        assert_eq!(geometry.blocks.len(), 4);
        assert!(geometry.blocks[2].bounds.height > LINE_HEIGHT * 2.0);
        assert!(geometry.blocks[3].bounds.height > LINE_HEIGHT * 2.0);
        assert!(!geometry.blocks[2].graphemes.is_empty());
        assert!(!geometry.blocks[3].graphemes.is_empty());
        assert!(
            geometry.blocks[3].bounds.y
                >= geometry.blocks[2].bounds.y + geometry.blocks[2].bounds.height
        );
    }

    #[test]
    fn pointer_drag_sets_selection_start_and_end() {
        let text = SelectableRichText::new([RichSpan::plain("A你e\u{301}Hello")]);
        let area = bounds(400.0, 20.0);
        assert!(text.pointer_down(caret_x(0), 8.0, area));
        assert!(text.pointer_move(caret_x(3), 8.0, area));
        let event = text.pointer_up(caret_x(3), 8.0, area);
        assert_eq!(
            event,
            Some(RichTextEvent::SelectionChanged(Some(
                TextSelectionSnapshot {
                    start: 0,
                    end: 3,
                    text: "A你e\u{301}".into(),
                }
            )))
        );
        let snapshot = text.selection_snapshot().expect("selection");
        assert_eq!(snapshot.start, 0);
        assert_eq!(snapshot.end, 3);
        assert_eq!(snapshot.text, "A你e\u{301}");
        assert_eq!(text.copy_snapshot(), Some(snapshot));
    }

    #[test]
    fn markdown_pointer_drag_selects_across_blocks() {
        let markdown = NativeMarkdown::from_blocks([
            MarkdownBlock::paragraph([MarkdownSpan::plain("Hello")]),
            MarkdownBlock::paragraph([MarkdownSpan::plain("World")]),
        ]);
        let area = bounds(400.0, 80.0);
        let world_y = 8.0 + LINE_HEIGHT + BLOCK_GAP;
        assert!(markdown.pointer_down(caret_x(0), 8.0, area));
        assert!(markdown.pointer_move(caret_x(5), world_y, area));
        let event = markdown.pointer_up(caret_x(5), world_y, area);
        assert_eq!(
            event,
            Some(RichTextEvent::SelectionChanged(Some(
                TextSelectionSnapshot {
                    start: 0,
                    end: 10,
                    text: "Hello\n\nWorld".into(),
                }
            )))
        );
    }

    #[test]
    fn link_hit_activates_on_click_without_drag() {
        let text = SelectableRichText::new([
            RichSpan::plain("See "),
            RichSpan::link("docs", "https://example.com/docs"),
        ]);
        let area = bounds(400.0, 20.0);
        let link_x = caret_x(5);
        assert_eq!(
            text.link_at(link_x, 8.0, area).as_deref(),
            Some("https://example.com/docs")
        );
        assert!(text.pointer_down(link_x, 8.0, area));
        assert_eq!(
            text.pointer_up(link_x, 8.0, area),
            Some(RichTextEvent::LinkActivated(Arc::from(
                "https://example.com/docs"
            )))
        );
        assert_eq!(text.selection_snapshot(), None);
    }

    #[test]
    fn empty_spans_are_omitted_from_geometry_and_selection() {
        let text = SelectableRichText::new([
            RichSpan::plain(""),
            RichSpan::plain("Hi"),
            RichSpan::plain(""),
        ]);
        let area = bounds(400.0, 20.0);
        let geometry = text.layout(area);
        assert_eq!(geometry.graphemes.len(), 2);
        assert_eq!(text.plain_text(), "Hi");
        assert!(text.pointer_down(caret_x(0), 8.0, area));
        assert!(text.pointer_move(caret_x(2), 8.0, area));
        assert_eq!(
            text.pointer_up(caret_x(2), 8.0, area),
            Some(RichTextEvent::SelectionChanged(Some(
                TextSelectionSnapshot {
                    start: 0,
                    end: 2,
                    text: "Hi".into(),
                }
            )))
        );
    }

    #[test]
    fn code_block_retains_language_and_highlight_request() {
        let markdown =
            NativeMarkdown::from_blocks([MarkdownBlock::code(Some("rust"), "fn main() {}")]);
        assert_eq!(markdown.blocks()[0].language(), Some("rust"));
        assert_eq!(
            markdown.blocks()[0].highlight_request(),
            Some(HighlightRequest::highlight("rust"))
        );
        assert_eq!(
            markdown.code_highlights(),
            [(0, HighlightRequest::highlight("rust"))]
        );
        let geometry = markdown.layout(bounds(400.0, 40.0));
        assert_eq!(geometry.blocks[0].language.as_deref(), Some("rust"));
        assert_eq!(markdown.plain_text(), "fn main() {}");

        let unlabeled = NativeMarkdown::from_blocks([MarkdownBlock::code(None, "plain")]);
        assert_eq!(unlabeled.blocks()[0].language(), None);
        assert_eq!(unlabeled.blocks()[0].highlight_request(), None);
        assert!(unlabeled.code_highlights().is_empty());
    }

    #[test]
    fn markdown_projects_plain_text_on_a_retained_leaf() {
        let mut context = AppContext::new();
        let markdown = context
            .create_component(
                document(),
                NativeMarkdown::from_blocks([
                    MarkdownBlock::heading(2, [MarkdownSpan::plain("Title")]),
                    MarkdownBlock::paragraph([MarkdownSpan::plain("Body")]),
                ]),
            )
            .unwrap();
        let id = markdown.stable_id();
        assert_eq!(context.world().text(id), Some("Title\n\nBody"));
        assert_eq!(
            context.world().widget_visual_as::<NativeMarkdownVisual>(id),
            Some(&NativeMarkdownVisual {
                blocks: NativeMarkdown::from_source("## Title\n\nBody")
                    .blocks
                    .into(),
                text: Arc::from("Title\n\nBody"),
                selection: None,
                code_presenter: Arc::from(crate::HIGHLIGHT_PRESENTER),
            })
        );
        assert!(context.world().highlight_request(id).is_none());
    }

    #[test]
    fn rich_text_projects_concatenated_spans_and_visual() {
        let mut context = AppContext::new();
        let text = context
            .create_component(
                document(),
                SelectableRichText::new([
                    RichSpan::plain("See "),
                    RichSpan::link("docs", "https://example.com/docs"),
                ]),
            )
            .unwrap();
        let id = text.stable_id();
        assert_eq!(context.world().text(id), Some("See docs"));
        assert_eq!(
            context
                .world()
                .widget_visual_as::<SelectableRichTextVisual>(id),
            Some(&SelectableRichTextVisual {
                text: Arc::from("See docs"),
                selection: None,
            })
        );
    }

    #[test]
    fn markdown_projects_selection_after_pointer_drag() {
        let mut context = AppContext::new();
        let markdown = context
            .create_component(
                document(),
                NativeMarkdown::from_blocks([
                    MarkdownBlock::paragraph([MarkdownSpan::plain("Hello")]),
                    MarkdownBlock::paragraph([MarkdownSpan::plain("World")]),
                ]),
            )
            .unwrap();
        let id = markdown.stable_id();
        let area = bounds(400.0, 80.0);
        let world_y = 8.0 + LINE_HEIGHT + BLOCK_GAP;
        context
            .update_component(markdown, |markdown, _| {
                assert!(markdown.pointer_down(caret_x(0), 8.0, area));
                assert!(markdown.pointer_move(caret_x(5), world_y, area));
                markdown.pointer_up(caret_x(5), world_y, area);
            })
            .unwrap();
        assert_eq!(
            context.world().widget_visual_as::<NativeMarkdownVisual>(id),
            Some(&NativeMarkdownVisual {
                blocks: NativeMarkdown::from_source("Hello\n\nWorld").blocks.into(),
                text: Arc::from("Hello\n\nWorld"),
                selection: Some((0, 10)),
                code_presenter: Arc::from(crate::HIGHLIGHT_PRESENTER),
            })
        );
        assert_eq!(context.world().text(id), Some("Hello\n\nWorld"));
    }

    #[test]
    fn selection_bounds_follow_the_selected_text_through_scrolling() {
        use crate::LayoutViewport;
        use nana_ui_core::LengthSpec;
        let mut context = AppContext::new();
        let scroll = context
            .create_component(
                document(),
                crate::ScrollView::new(crate::ScrollAxes::Vertical)
                    .with_layout(|layout| layout.height = Some(LengthSpec::Px(200.0))),
            )
            .unwrap();
        let column = context
            .create_detached_component(document(), crate::Stack::column(0.0))
            .unwrap();
        let spacer = context
            .create_detached_component(
                document(),
                crate::Stack::column(0.0).height(LengthSpec::Px(100.0)),
            )
            .unwrap();
        let markdown = context
            .create_detached_component(
                document(),
                NativeMarkdown::from_blocks([
                    MarkdownBlock::paragraph([MarkdownSpan::plain("Hello")]),
                    MarkdownBlock::paragraph([MarkdownSpan::plain("World")]),
                ]),
            )
            .unwrap();
        let tail = context
            .create_detached_component(
                document(),
                crate::Stack::column(0.0).height(LengthSpec::Px(400.0)),
            )
            .unwrap();
        context.append_child(column, spacer).unwrap();
        context.append_child(column, markdown).unwrap();
        context.append_child(column, tail).unwrap();
        context.append_child(scroll, column).unwrap();
        let relayout = |context: &mut AppContext| {
            context
                .layout_document(document(), LayoutViewport::new(400.0, 200.0))
                .unwrap();
        };
        relayout(&mut context);
        let id = markdown.stable_id();
        assert_eq!(context.world().text_selection_bounds(id), None);

        let area = bounds(400.0, 80.0);
        let second_line = 8.0 + LINE_HEIGHT + BLOCK_GAP;
        context
            .update_component(markdown, |markdown, _| {
                markdown.pointer_down(caret_x(1), second_line, area);
                markdown.pointer_move(caret_x(3), second_line, area);
                markdown.pointer_up(caret_x(3), second_line, area);
            })
            .unwrap();
        relayout(&mut context);
        let text = context.world().canonical_layout_box(id).unwrap();
        let selected = context.world().text_selection_bounds(id).unwrap();
        assert!(
            selected.y > text.y + LINE_HEIGHT,
            "{selected:?} in {text:?}"
        );
        assert!(selected.x > text.x && selected.width > 0.0);
        assert!(selected.height <= LINE_HEIGHT + 1.0);

        context
            .scroll_to(scroll, crate::ScrollOffset { x: 0.0, y: 60.0 })
            .unwrap();
        relayout(&mut context);
        let scrolled = context.world().text_selection_bounds(id).unwrap();
        assert!(
            (scrolled.y - (selected.y - 60.0)).abs() < 0.5,
            "{scrolled:?}"
        );
        assert_eq!(scrolled.x, selected.x);
    }

    #[test]
    fn rich_text_projects_selection_after_pointer_drag() {
        let mut context = AppContext::new();
        let text = context
            .create_component(
                document(),
                SelectableRichText::new([RichSpan::plain("A你e\u{301}Hello")]),
            )
            .unwrap();
        let id = text.stable_id();
        let area = bounds(400.0, 20.0);
        context
            .update_component(text, |text, _| {
                assert!(text.pointer_down(caret_x(0), 8.0, area));
                assert!(text.pointer_move(caret_x(3), 8.0, area));
                text.pointer_up(caret_x(3), 8.0, area);
            })
            .unwrap();
        assert_eq!(
            context
                .world()
                .widget_visual_as::<SelectableRichTextVisual>(id),
            Some(&SelectableRichTextVisual {
                text: Arc::from("A你e\u{301}Hello"),
                selection: Some((0, 3)),
            })
        );
        assert_eq!(context.world().text(id), Some("A你e\u{301}Hello"));
    }

    #[test]
    fn diagram_fences_name_themselves_from_acc_title_and_description() {
        let parsed = NativeMarkdown::from_source(
            "```mermaid\nflowchart LR\naccTitle: Login flow\naccDescr: How a user signs in\nA-->B-->C\n```",
        );
        let mut context = AppContext::new();
        let markdown = context.create_component(document(), parsed).unwrap();
        context.assemble_markdown(markdown).unwrap();
        let child = context.world().node(markdown.stable_id()).unwrap().children[0];
        let state = context.world().accessibility(child).unwrap();
        assert_eq!(state.label.as_deref(), Some("Login flow"));
        assert_eq!(state.description.as_deref(), Some("How a user signs in"));
    }

    #[test]
    fn assemble_markdown_creates_fence_children_for_mermaid_math_and_code() {
        let source = concat!(
            "```mermaid\n",
            "flowchart LR\n",
            "A-->B\n",
            "```\n\n",
            "```math\n",
            "\\frac{1}{2}\n",
            "```\n\n",
            "```rust\n",
            "fn main() {}\n",
            "```\n",
        );
        let parsed = NativeMarkdown::from_source(source);
        assert!(matches!(
            parsed.blocks(),
            [
                MarkdownBlock::Mermaid(mermaid),
                MarkdownBlock::DisplayMath(math),
                MarkdownBlock::Code {
                    language: Some(language),
                    source: rust,
                },
            ] if mermaid == "flowchart LR\nA-->B"
                && math == "\\frac{1}{2}"
                && language == "rust"
                && rust == "fn main() {}"
        ));
        assert_eq!(
            parsed.blocks()[0].fence_highlight_request(),
            Some(HighlightRequest::new(
                NativeMarkdown::MERMAID_PRESENTER,
                "flowchart LR\nA-->B"
            ))
        );
        assert_eq!(
            parsed.blocks()[1].fence_highlight_request(),
            Some(HighlightRequest::new(
                NativeMarkdown::MATH_PRESENTER,
                "\\frac{1}{2}"
            ))
        );
        assert_eq!(
            parsed.blocks()[2].fence_highlight_request(),
            Some(HighlightRequest::highlight("rust"))
        );

        let mut context = AppContext::new();
        let markdown = context.create_component(document(), parsed).unwrap();
        let id = markdown.stable_id();
        assert!(context.world().node(id).unwrap().children.is_empty());
        assert!(
            context
                .world()
                .widget_visual_as::<NativeMarkdownVisual>(id)
                .is_some(),
            "project stays a NativeMarkdown leaf before assembly"
        );

        context.assemble_markdown(markdown).unwrap();
        let children = context.world().node(id).unwrap().children;
        assert_eq!(children.len(), 3);
        assert_eq!(
            context.read(markdown, |markdown| markdown.fence_children().to_vec()),
            Ok(children.clone())
        );
        assert_eq!(
            context.world().text(children[0]),
            Some("flowchart LR\nA-->B")
        );
        assert_eq!(
            context.world().highlight_request(children[0]),
            Some(&HighlightRequest::new(
                NativeMarkdown::MERMAID_PRESENTER,
                "flowchart LR\nA-->B"
            ))
        );
        let diagram = context.world().accessibility(children[0]).unwrap();
        assert_eq!(diagram.label.as_deref(), Some("图表"));
        assert_eq!(diagram.description.as_deref(), Some("2 个节点，1 条连线"));
        assert!(
            context
                .world()
                .accessibility(children[1])
                .unwrap()
                .label
                .is_none()
        );
        assert_eq!(context.world().text(children[1]), Some("\\frac{1}{2}"));
        assert_eq!(
            context.world().highlight_request(children[1]),
            Some(&HighlightRequest::new(
                NativeMarkdown::MATH_PRESENTER,
                "\\frac{1}{2}"
            ))
        );
        assert_eq!(context.world().text(children[2]), Some("fn main() {}"));
        assert_eq!(
            context.world().highlight_request(children[2]),
            Some(&HighlightRequest::highlight("rust"))
        );
        for child in &children {
            assert!(
                context
                    .world()
                    .node_style(*child)
                    .is_some_and(|style| style.layout.hidden),
                "fence children are identity slots and must not paint Scene text"
            );
            assert!(context.world().widget_visual_ref(*child).is_none());
            assert_eq!(
                context.world().interaction(*child),
                Some(InteractionState {
                    pointer_events: false,
                    focusable: false,
                })
            );
        }
        assert!(
            matches!(
                context.world().widget_visual_as::<NativeMarkdownVisual>(id),
                Some(NativeMarkdownVisual { text, selection: None, .. })
                    if text.contains("flowchart LR")
                        && text.contains("\\frac{1}{2}")
                        && text.contains("fn main() {}")
            ),
            "parent still projects NativeMarkdownVisual after assembly"
        );
        assert!(context.world().highlight_request(id).is_none());
    }

    #[test]
    fn assemble_markdown_drops_stale_fence_children() {
        let mut context = AppContext::new();
        let markdown = context
            .create_component(
                document(),
                NativeMarkdown::from_source(concat!(
                    "```mermaid\n",
                    "flowchart LR\n",
                    "A-->B\n",
                    "```\n\n",
                    "```math\n",
                    "\\frac{1}{2}\n",
                    "```\n\n",
                    "```rust\n",
                    "fn main() {}\n",
                    "```\n",
                )),
            )
            .unwrap();
        context.assemble_markdown(markdown).unwrap();
        let first_children = context.world().node(markdown.stable_id()).unwrap().children;
        assert_eq!(first_children.len(), 3);
        let dropped = first_children[2];

        context
            .update_component(markdown, |markdown, _| {
                *markdown = NativeMarkdown::from_source(concat!(
                    "```mermaid\n",
                    "flowchart LR\n",
                    "A-->B\n",
                    "```\n\n",
                    "```math\n",
                    "\\frac{1}{2}\n",
                    "```\n",
                ));
            })
            .unwrap();
        context.assemble_markdown(markdown).unwrap();

        let children = context.world().node(markdown.stable_id()).unwrap().children;
        assert_eq!(children.len(), 2);
        assert_eq!(
            context.world().highlight_request(children[0]),
            Some(&HighlightRequest::new(
                NativeMarkdown::MERMAID_PRESENTER,
                "flowchart LR\nA-->B"
            ))
        );
        assert_eq!(
            context.world().highlight_request(children[1]),
            Some(&HighlightRequest::new(
                NativeMarkdown::MATH_PRESENTER,
                "\\frac{1}{2}"
            ))
        );
        assert_eq!(
            context.world().text(children[0]),
            Some("flowchart LR\nA-->B")
        );
        assert_eq!(context.world().text(children[1]), Some("\\frac{1}{2}"));
        for child in &children {
            assert!(
                context
                    .world()
                    .node_style(*child)
                    .is_some_and(|style| style.layout.hidden),
                "re-assembled fence children stay hidden identity slots"
            );
            assert!(context.world().widget_visual_ref(*child).is_none());
        }
        assert!(
            !children.contains(&dropped),
            "removed rust fence must not remain a child"
        );
        assert!(
            !context.world().contains(dropped),
            "stale fence children must be despawned"
        );
        assert!(
            context
                .world()
                .widget_visual_as::<NativeMarkdownVisual>(markdown.stable_id())
                .is_some()
        );
    }
}
