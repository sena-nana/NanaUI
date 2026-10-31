//! 主题文本着色器的保留字形快照，复用唯一文本引擎并保留原生对象绘制顺序。
use super::{AffineTransform, ScenePrimitive, ScenePrimitiveKind, SceneRect, SceneTextSpan};

/// A glyph record exported for a dialogue/theme shader.
///
/// The atlas remains owned by `nana-ui`; this record is the stable,
/// backend-neutral coverage geometry a consumer needs to build a mask or
/// provide an alternate text pass. `position` and `advance` are in the
/// retained layout's physical-pixel space, before the scene transform.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SceneTextShaderGlyph {
    pub glyph_id: u32,
    pub cluster: u32,
    pub cluster_end: u32,
    pub position: [f32; 2],
    pub advance: [f32; 2],
    pub offset: [f32; 2],
    pub flags: nana_text::GlyphFlags,
    pub line: u32,
    pub run: u32,
    /// The resolved paint color for this glyph's source range, when known.
    pub color: Option<nana_ui_core::PaintColor>,
}

/// An inline object occupying a place in the text flow. Theme shaders should
/// leave these regions to the ordinary image/object painter.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SceneTextShaderInlineObject {
    pub id: u64,
    pub offset: usize,
    pub line: u32,
    pub rect: nana_text::TextRect,
}

/// Backend-neutral input for a custom dialogue text shader.
///
/// This is a snapshot of one retained text primitive. The source `layout_id`
/// changes whenever shaping or line layout changes; paint-only changes update
/// colors without invalidating glyph geometry. The ordinary scene primitive
/// must still be painted so text, images and inline objects retain document
/// order.
#[derive(Debug, Clone, PartialEq)]
pub struct SceneTextShaderInput {
    pub layout_id: nana_text::TextLayoutId,
    pub content: nana_ui_runtime::TextValue,
    pub bounds: SceneRect,
    pub transform: AffineTransform,
    pub opacity: f32,
    pub base_color: Option<nana_ui_core::PaintColor>,
    pub spans: Vec<SceneTextSpan>,
    pub glyphs: Vec<SceneTextShaderGlyph>,
    pub inline_objects: Vec<SceneTextShaderInlineObject>,
}

impl ScenePrimitive {
    /// Return the retained glyph coverage input for a text primitive.
    ///
    /// Host-provided shapers without a retained `nana-text` layout return
    /// `None`; their normal text path remains unchanged.
    pub fn text_shader_input(&self) -> Option<SceneTextShaderInput> {
        let ScenePrimitiveKind::Text {
            content,
            color,
            spans,
            layout: Some(retained),
            ..
        } = &self.kind
        else {
            return None;
        };
        let layout = &retained.layout;
        let glyph_color = |cluster: usize| {
            spans
                .iter()
                .find(|span| span.start <= cluster && cluster < span.end)
                .map(|span| span.color.clone())
                .or_else(|| color.clone())
        };
        let mut glyphs = Vec::with_capacity(layout.glyph_count());
        for line in &layout.lines {
            for (run_index, run) in layout.line_runs(line).iter().enumerate() {
                for (cell_x, glyph) in run.glyph_cells() {
                    let position = if run.orientation.is_vertical() {
                        [
                            line.metrics.baseline_y_px + glyph.offset_y_px,
                            cell_x + glyph.offset_x_px,
                        ]
                    } else {
                        [
                            cell_x + glyph.offset_x_px,
                            line.metrics.baseline_y_px - glyph.offset_y_px,
                        ]
                    };
                    glyphs.push(SceneTextShaderGlyph {
                        glyph_id: glyph.glyph_id,
                        cluster: glyph.cluster,
                        cluster_end: glyph.cluster_end,
                        position,
                        advance: [glyph.advance_px, glyph.advance_y_px],
                        offset: [glyph.offset_x_px, glyph.offset_y_px],
                        flags: glyph.flags,
                        line: line.index,
                        run: run_index as u32,
                        color: glyph_color(glyph.cluster as usize),
                    });
                }
            }
        }
        Some(SceneTextShaderInput {
            layout_id: retained.id,
            content: content.clone(),
            bounds: self.bounds,
            transform: self.transform,
            opacity: self.opacity,
            base_color: color.clone(),
            spans: spans.clone(),
            glyphs,
            inline_objects: layout
                .objects
                .iter()
                .map(|object| SceneTextShaderInlineObject {
                    id: object.id,
                    offset: object.offset,
                    line: object.line,
                    rect: object.rect,
                })
                .collect(),
        })
    }
}
