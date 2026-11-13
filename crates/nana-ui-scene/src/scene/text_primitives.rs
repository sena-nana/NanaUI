//! Primitives built from text runs: span backgrounds, editor marks, inline
//! objects and rich fill spans.

use super::*;
use nana_ui_core::sort::CompactSort as _;

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

/// A rich text's span backgrounds, from the very layout its text is drawn
/// from: each span fills its run on every line it reaches, as CSS paints an
/// inline box's `background-color`. Under any selection and the glyphs.
pub(super) fn rich_span_background_primitives(
    context: &VisualPrimitiveContext<'_>,
    bounds: SceneRect,
    vertical: TextVerticalAlignment,
    retained: &nana_ui_runtime::RetainedTextLayout,
    rich: &nana_ui_core::RichText,
) -> Vec<ScenePrimitive> {
    let layout = &retained.layout;
    let (page_width, top) = text_frame(bounds, vertical, retained);
    rich.spans()
        .iter()
        .filter_map(|(range, style)| Some((range, style.paint.background_color?)))
        .take(DOCUMENT_SELECTION_INDEX)
        .enumerate()
        .filter_map(|(index, (range, color))| {
            let rects = layout.selection_rects(range);
            if rects.is_empty() {
                return None;
            }
            let mut fill = visual_quad_batch(
                context,
                collection_slot(DOCUMENT_TEXT_SELECTION, index),
                rects.into_iter().map(|rect| {
                    let rect = layout.page_rect(rect, page_width);
                    SceneRect {
                        x: bounds.x + rect.x,
                        y: top + rect.y,
                        width: rect.width,
                        height: rect.height,
                    }
                }),
                VisualQuadStyle::solid(color.to_srgb()),
            );
            // The authored colour, so the painter resolves it as it does a
            // glyph fill.
            if let ScenePrimitiveKind::QuadBatch { background, .. } = &mut fill.kind {
                *background = Some(color);
            }
            Some(fill)
        })
        .collect()
}

/// A rich text editor's selection (under the glyphs) and caret (over them),
/// from the very layout its text is drawn from.
#[allow(clippy::too_many_arguments)]
pub(super) fn rich_editor_mark_primitives(
    context: &VisualPrimitiveContext<'_>,
    bounds: SceneRect,
    vertical: TextVerticalAlignment,
    retained: &nana_ui_runtime::RetainedTextLayout,
    marks: &nana_ui_runtime::RichEditorMarks,
    selection_color: [f32; 4],
    selection_paint_color: Option<nana_ui_core::PaintColor>,
    caret_color: [f32; 4],
    caret_paint_color: Option<nana_ui_core::PaintColor>,
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
                let mut quad = visual_quad(
                    context,
                    collection_slot(DOCUMENT_TEXT_SELECTION, RICH_EDITOR_SELECTION_BASE + index),
                    SceneRect {
                        x: bounds.x + rect.x,
                        y: top + rect.y,
                        width: rect.width,
                        height: rect.height,
                    },
                    VisualQuadStyle::solid(selection_color),
                );
                if let ScenePrimitiveKind::Quad { background, .. } = &mut quad.kind {
                    *background = selection_paint_color.or(*background);
                }
                quad
            })
            .collect()
    };
    let caret = marks.caret.and_then(|byte| {
        let (x, line_top, height) = nana_ui_runtime::rich_editor_caret_box(layout, byte)?;
        let mut quad = visual_quad(
            context,
            collection_slot(TEXT_EDITOR_CARET, 0),
            SceneRect {
                x: bounds.x + x - 0.5,
                y: top + line_top,
                width: 1.5,
                height,
            },
            VisualQuadStyle::solid(caret_color),
        );
        if let ScenePrimitiveKind::Quad { background, .. } = &mut quad.kind {
            *background = caret_paint_color.or(*background);
        }
        Some(quad)
    });
    (selection, caret)
}

/// An inline object a presenting text node carries, as it was laid out.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct GlyphObject {
    pub slot: u64,
    pub ordinal: u32,
    pub center: [f32; 2],
    pub target: SceneRect,
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
pub(super) fn inline_object_primitives(
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
            nana_ui_core::RichObjectContent::Tag { .. }
            | nana_ui_core::RichObjectContent::Label { .. } => {
                let Some(label) = layout
                    .labels
                    .iter()
                    .find(|label| label.offset == placed.offset)
                else {
                    continue;
                };
                let page = |rect| {
                    let rect = layout.page_rect(rect, box_width.max(bounds.width));
                    SceneRect {
                        x: bounds.x + rect.x,
                        y: top + rect.y,
                        width: rect.width,
                        height: rect.height,
                    }
                };
                let pill = page(label.rect);
                let (fill, radius) = match &object.content {
                    nana_ui_core::RichObjectContent::Label { style, .. } => (
                        style.fill,
                        style
                            .radius_px
                            .filter(|r| r.is_finite() && *r >= 0.0)
                            .unwrap_or(pill.height * 0.5)
                            .min(pill.height * 0.5),
                    ),
                    nana_ui_core::RichObjectContent::Tag { kind, .. } => {
                        (tag_color(*kind), pill.height * 0.5)
                    }
                    _ => unreachable!("matched above"),
                };
                let mut style = VisualQuadStyle::solid(fill);
                style.corner_radius = corner_radii(radius);
                out.push(visual_quad(
                    context,
                    collection_slot(DOCUMENT_TEXT_SELECTION, RICH_TAG_BASE + index),
                    pill,
                    style,
                ));
                // A label's picture sits in the square its layout kept.
                if let (Some(style), Some(icon)) = (object.label_style(), label.icon)
                    && let Some(source) = &style.icon
                {
                    let icon = page(icon);
                    let radius = if style.icon_round {
                        icon.height * 0.5
                    } else {
                        icon.height * 0.2
                    };
                    out.push(ScenePrimitive {
                        id: PrimitiveId {
                            node: context.node,
                            slot: collection_slot(TEXT_INLINE_OBJECTS, index),
                        },
                        node: context.node,
                        bounds: icon,
                        transform: context.transform,
                        clips: Arc::clone(context.clips),
                        opacity: context.opacity,
                        z_index: context.z_index,
                        document_order: context.document_order,
                        kind: custom_paint::image_quad(
                            source,
                            nana_ui_runtime::ImageFit::Cover,
                            nana_ui_core::ImageSampling::default(),
                            corner_radii(radius),
                        ),
                    });
                }
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
    out.compact_sort_by_key(|span| (span.start, span.end));
    out
}
