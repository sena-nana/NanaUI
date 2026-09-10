//! rich-text geometry from committed node data.

use super::*;

pub(in crate::world) fn selectable_text_regions(
    content: LayoutBox,
    text: &Arc<str>,
    selection: Option<(usize, usize)>,
    style: &ComputedStyle,
    palette: &SemanticPalette,
) -> (crate::ComponentTextRegion, Vec<LayoutBox>, [f32; 4]) {
    let region = crate::ComponentTextRegion {
        bounds: content,
        content: Arc::clone(text),
        color: Some(style.color.unwrap_or_else(|| palette.text.as_rgba_array())),
        font_size: style.font_size,
        font_weight: style.font_weight,
    };
    let highlights = if selection.is_some() {
        vec![content]
    } else {
        Vec::new()
    };
    (region, highlights, palette.accent_soft.as_rgba_array())
}

impl UiWorld {
    pub(crate) fn markdown_layout(
        &self,
        id: StableNodeId,
        blocks: &[crate::MarkdownBlock],
        bounds: LayoutBox,
    ) -> crate::rich_text::MarkdownGeometry {
        let Some(style) = self.computed_style(id) else {
            return crate::rich_text::layout_markdown(blocks, bounds);
        };
        crate::rich_text::layout_markdown_measured(
            blocks,
            bounds,
            Some(&|grapheme, size, weight| {
                let mut measured = style.clone();
                measured.font_size = size;
                measured.font_weight = Some(weight);
                let advances = grapheme
                    .chars()
                    .map(|ch| {
                        self.glyph_cache.peek(ch, &measured).or_else(|| {
                            if weight == 400 {
                                measured.font_weight = style.font_weight;
                                let value = self.glyph_cache.peek(ch, &measured);
                                measured.font_weight = Some(weight);
                                value
                            } else {
                                None
                            }
                        })
                    })
                    .collect::<Option<Vec<_>>>();
                advances
                    .map(|values| values.into_iter().sum())
                    .unwrap_or_else(|| crate::markdown_drawing::text_advance(grapheme, size))
            }),
        )
    }
}
