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
