//! Component text primitives: editor scroll, select handles, text spans and
//! icon alignment beside text.

use super::*;

/// What of where a node stands its build read
/// ([`UiScene::icon_y_aligned_to_adjacent_text`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum PlacementRead {
    None,
    /// Its own offset in its parent's coordinates.
    Offset,
    /// Where it stands on the page, or where another node does.
    Page,
}

pub(super) fn surface_corner_radii(
    style: &nana_ui_core::LayoutStyle,
    width: f32,
    height: f32,
) -> [f32; 4] {
    let mut radii = style.resolved_border_radii(width, height);
    if let Some(ClipPath::Inset(inset)) = style.paint.clip_path.as_ref() {
        let round = inset.resolve_round(width, height);
        if round > 0.0 {
            radii = radii.map(|radius| radius.max(round));
        }
    }
    radii
}

pub(super) fn scene_text_spans(
    node: &ExtractedNode,
    _region: Option<&ComponentTextRegion>,
    content: &str,
) -> Vec<SceneTextSpan> {
    scene_text_spans_with_adopt(node, _region, content, false)
}

pub(super) fn scene_text_spans_with_adopt(
    node: &ExtractedNode,
    _region: Option<&ComponentTextRegion>,
    content: &str,
    adopt_node_text_spans: bool,
) -> Vec<SceneTextSpan> {
    if node.text_spans.is_empty() && node.rich_text.is_none() {
        return Vec::new();
    }
    // TextInput 主文本区域：span 由 extraction 按 TextInput 呈现内容
    // （折叠态经显示视图重投后的显示串）空间产出，直接按 content 校验
    // 边界。其余区域（行号标签、钉住行、Select 标题、普通文本节点等）
    // 不承载该空间，维持值空间等值守卫——内容与 span 空间不一致时整批
    // 丢弃。
    let editor_main_text = adopt_node_text_spans;
    if !editor_main_text {
        let Some(source) = node.text.as_ref() else {
            return Vec::new();
        };
        if source.value != content {
            return Vec::new();
        }
    }
    let spans = node
        .text_spans
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
        .collect();
    rich_fill_spans(content, node.rich_text.as_ref(), spans)
}

impl UiScene {
    /// Skip duplicate `#text` children when the host already paints that
    /// string. Independent element children, such as list rows inside a Card,
    /// keep their own labels.
    pub(super) fn parent_already_paints_text(&self, node: &ExtractedNode) -> bool {
        if node
            .extracted_layout
            .is_some_and(|layout| layout.atomic_inline)
        {
            return false;
        }

        if !matches!(&*node.kind, NodeKind::Text) {
            return false;
        }
        let Some(parent) = node.parent.and_then(|id| self.nodes.get(&id)) else {
            return false;
        };
        if let Some(w) = parent.widget_paint.as_deref() {
            match &w.child_text_suppression {
                nana_ui_runtime::WidgetChildTextSuppression::RespectOwnsNodeText => {
                    if w.owns_node_text {
                        return true;
                    }
                }
                nana_ui_runtime::WidgetChildTextSuppression::Suppress => return true,
                nana_ui_runtime::WidgetChildTextSuppression::Allow => return false,
                nana_ui_runtime::WidgetChildTextSuppression::Matching(expected) => {
                    return node
                        .text
                        .as_ref()
                        .is_some_and(|text| text.value == expected.as_ref());
                }
            }
        }
        parent
            .text
            .as_ref()
            .is_some_and(|text| !text.value.is_empty())
    }

    /// Compact leading glyphs share the parent (or own) text line-box center.
    ///
    /// The second half of the answer is what of where the node stands it
    /// read: nothing, its own offset in its parent's coordinates (it is built
    /// again when it is the root of a move, [`Self::offset_bound`]), or the
    /// page (when it or a box above it moves, [`Self::origin_bound`]).
    pub(super) fn icon_y_aligned_to_adjacent_text(
        &self,
        node: &ExtractedNode,
        icon_bounds: SceneRect,
        extent: f32,
    ) -> (f32, PlacementRead) {
        let geometric = icon_bounds.y + (icon_bounds.height - extent) / 2.0;
        if node.layout.height > extent + 0.5 || node.layout.width > extent + 0.5 {
            return (geometric, PlacementRead::None);
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
                        || parent
                            .widget_paint
                            .as_deref()
                            .is_some_and(|w| w.aligns_child_icon_to_text) =>
                {
                    parent
                }
                _ => return (geometric, PlacementRead::None),
            }
        };
        // In the space the host's record is in.
        let mut text_box = host
            .widget_paint
            .as_deref()
            .filter(|w| w.aligns_child_icon_to_text)
            .and_then(|w| w.node_text_bounds)
            .unwrap_or_else(|| Self::record_box(host));
        // Into this node's: a child measured from its parent stands at its
        // local box in the parent's coordinates, wherever the two stand.
        let mut read = PlacementRead::None;
        if host.id != node.id {
            if node.layout_frame == nana_ui_runtime::LayoutFrame::Parent
                && !record_on_page(host)
                && !record_on_page(node)
            {
                text_box.y -= node.local_layout.y;
                read = PlacementRead::Offset;
            } else {
                text_box.y += self.paint_origin(host)[1] - self.paint_origin(node)[1];
                read = PlacementRead::Page;
            }
        }
        let centered = matches!(
            host.source_style.text_vertical_alignment,
            TextVerticalAlignment::Center
        );
        (
            icon_y_on_text_glyph_center(
                text_box.y,
                text_box.height,
                host.style.font_size,
                host.style.line_height,
                centered,
                extent,
            ),
            read,
        )
    }
}
