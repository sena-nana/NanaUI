use cosmic_text::{
    Align, Attrs, Buffer, Color, Family, FontSystem, Metrics, Shaping, SwashCache, Weight, Wrap,
};
use nana_ui_core::LineHeightSpec;
use nana_ui_runtime::{TextHorizontalAlignment, TextShaping, TextVerticalAlignment};
use nana_ui_scene::SceneTextSpan;

use super::clip::LogicalRect;
use super::color::{to_rgba8, with_opacity};
use crate::PhysicalRect;

pub(super) struct TextPipeline {
    font_system: FontSystem,
    #[allow(dead_code)]
    cache: cryoglyph::Cache,
    atlas: cryoglyph::TextAtlas,
    viewport: cryoglyph::Viewport,
    swash: SwashCache,
    renderers: Vec<cryoglyph::TextRenderer>,
    buffers: Vec<Buffer>,
}

pub(super) struct PreparedText {
    pub index: usize,
}

impl TextPipeline {
    pub(super) fn new(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        format: wgpu::TextureFormat,
    ) -> Self {
        let cache = cryoglyph::Cache::new(device);
        let atlas = cryoglyph::TextAtlas::with_color_mode(
            device,
            queue,
            &cache,
            format,
            cryoglyph::ColorMode::Accurate,
        );
        let viewport = cryoglyph::Viewport::new(device, &cache);
        Self {
            font_system: crate::nana_text::nana_font_system(),
            cache,
            atlas,
            viewport,
            swash: SwashCache::new(),
            renderers: Vec::new(),
            buffers: Vec::new(),
        }
    }

    pub(super) fn begin_frame(&mut self, queue: &wgpu::Queue, physical_size: [u32; 2]) {
        self.buffers.clear();
        self.viewport.update(
            queue,
            cryoglyph::Resolution {
                width: physical_size[0].max(1),
                height: physical_size[1].max(1),
            },
        );
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn prepare(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        bounds: LogicalRect,
        clip: LogicalRect,
        scale_factor: f32,
        content: &str,
        color: Option<[f32; 4]>,
        size: f32,
        weight: Option<u16>,
        family: Option<&str>,
        line_height: Option<LineHeightSpec>,
        wrap: bool,
        ellipsis: bool,
        shaping: TextShaping,
        horizontal: TextHorizontalAlignment,
        vertical: TextVerticalAlignment,
        spans: &[SceneTextSpan],
        opacity: f32,
    ) -> Option<PreparedText> {
        if content.is_empty() || bounds.width <= 0.0 || bounds.height <= 0.0 {
            return None;
        }
        let scale = if scale_factor.is_finite() && scale_factor > 0.0 {
            scale_factor
        } else {
            1.0
        };
        let size = size.max(f32::MIN_POSITIVE);
        let line_height = match line_height {
            Some(LineHeightSpec::Relative(value)) => size * value,
            Some(LineHeightSpec::Absolute(value)) => value,
            None => size * 1.2,
        }
        .max(f32::MIN_POSITIVE);
        // Shape in physical px and keep TextArea.scale = 1 so glyphs are not double-scaled.
        let physical_size = size * scale;
        let physical_line_height = line_height * scale;
        let physical_width = bounds.width.max(0.0) * scale;
        let physical_height = bounds.height.max(line_height) * scale;
        let default_color = with_opacity(color.unwrap_or([0.0, 0.0, 0.0, 1.0]), opacity);
        let attrs = text_attrs(family, weight);
        let shaping = match shaping {
            TextShaping::Auto if content.is_ascii() => Shaping::Basic,
            TextShaping::Auto | TextShaping::Advanced => Shaping::Advanced,
        };
        let align = match horizontal {
            TextHorizontalAlignment::Start => None,
            TextHorizontalAlignment::Center => Some(Align::Center),
            TextHorizontalAlignment::End => Some(Align::Right),
        };
        let mut buffer = Buffer::new(
            &mut self.font_system,
            Metrics::new(physical_size, physical_line_height),
        );
        buffer.set_size(Some(physical_width), Some(physical_height));
        buffer.set_wrap(if wrap { Wrap::Word } else { Wrap::None });
        buffer.set_ellipsize(if ellipsis {
            cosmic_text::Ellipsize::End(cosmic_text::EllipsizeHeightLimit::Height(physical_height))
        } else {
            cosmic_text::Ellipsize::None
        });
        let painted = presentation_spans(content, spans, default_color, opacity);
        if painted.len() > 1 || painted.first().is_some_and(|span| span.1 != default_color) {
            let rich = painted
                .iter()
                .map(|(text, color)| (*text, attrs.clone().color(rgba8_color(*color))))
                .collect::<Vec<_>>();
            buffer.set_rich_text(rich, &attrs, shaping, align);
        } else {
            buffer.set_text(content, &attrs, shaping, align);
        }
        buffer.shape_until_scroll(&mut self.font_system, false);
        let (_, laid_out_height) = measure(&buffer);
        let [left, top] = text_box_origin(
            LogicalRect {
                x: bounds.x * scale,
                y: bounds.y * scale,
                width: physical_width,
                height: physical_height,
            },
            vertical,
            laid_out_height,
        );
        let text_bounds = cryoglyph::TextBounds {
            left: (clip.x * scale).round() as i32,
            top: (clip.y * scale).round() as i32,
            right: ((clip.x + clip.width) * scale).round() as i32,
            bottom: ((clip.y + clip.height) * scale).round() as i32,
        };
        let index = self.buffers.len();
        self.buffers.push(buffer);
        if self.renderers.len() <= index {
            self.renderers.push(cryoglyph::TextRenderer::new(
                &mut self.atlas,
                device,
                wgpu::MultisampleState::default(),
                None,
            ));
        }
        let area = cryoglyph::TextArea {
            text: self.buffers[index].layout_runs(),
            left,
            top,
            scale: 1.0,
            bounds: text_bounds,
            default_color: rgba8_color(default_color),
        };
        let result = self.renderers[index].prepare(
            device,
            queue,
            encoder,
            &mut self.font_system,
            &mut self.atlas,
            &self.viewport,
            [area],
            &mut self.swash,
        );
        if matches!(result, Err(cryoglyph::PrepareError::AtlasFull)) {
            self.atlas.trim();
            let area = cryoglyph::TextArea {
                text: self.buffers[index].layout_runs(),
                left,
                top,
                scale: 1.0,
                bounds: text_bounds,
                default_color: rgba8_color(default_color),
            };
            let _ = self.renderers[index].prepare(
                device,
                queue,
                encoder,
                &mut self.font_system,
                &mut self.atlas,
                &self.viewport,
                [area],
                &mut self.swash,
            );
        }
        Some(PreparedText { index })
    }

    pub(super) fn draw(
        &self,
        pass: &mut wgpu::RenderPass<'_>,
        prepared: &PreparedText,
        scissor: PhysicalRect,
    ) {
        let Some(renderer) = self.renderers.get(prepared.index) else {
            return;
        };
        pass.set_scissor_rect(scissor.x, scissor.y, scissor.width, scissor.height);
        let _ = renderer.render(&self.atlas, &self.viewport, pass);
    }
}

fn presentation_spans<'a>(
    content: &'a str,
    spans: &'a [SceneTextSpan],
    default: [f32; 4],
    opacity: f32,
) -> Vec<(&'a str, [f32; 4])> {
    let mut painted = Vec::new();
    let mut cursor = 0usize;
    for span in spans {
        if span.start > content.len()
            || span.end > content.len()
            || span.start >= span.end
            || !content.is_char_boundary(span.start)
            || !content.is_char_boundary(span.end)
        {
            continue;
        }
        if span.start > cursor {
            painted.push((&content[cursor..span.start], default));
        }
        painted.push((
            &content[span.start..span.end],
            with_opacity(span.color, opacity),
        ));
        cursor = span.end;
    }
    if cursor < content.len() {
        painted.push((&content[cursor..], default));
    }
    painted
}

fn measure(buffer: &Buffer) -> (f32, f32) {
    buffer
        .layout_runs()
        .fold((0.0, 0.0), |(width, height), run| {
            (run.line_w.max(width), height + run.line_height)
        })
}

fn text_box_origin(
    bounds: LogicalRect,
    vertical: TextVerticalAlignment,
    laid_out_height: f32,
) -> [f32; 2] {
    let top = match vertical {
        TextVerticalAlignment::Top => bounds.y,
        TextVerticalAlignment::Center => bounds.y + (bounds.height - laid_out_height) * 0.5,
        TextVerticalAlignment::Bottom => bounds.y + bounds.height - laid_out_height,
    };
    [bounds.x, top]
}

fn text_attrs(family: Option<&str>, weight: Option<u16>) -> Attrs<'static> {
    let family = family.unwrap_or_default().to_ascii_lowercase();
    let family = if family.contains("mono") {
        Family::Monospace
    } else {
        Family::SansSerif
    };
    Attrs::new()
        .family(family)
        .weight(match weight.unwrap_or(400) {
            0..=149 => Weight::THIN,
            150..=249 => Weight::EXTRA_LIGHT,
            250..=349 => Weight::LIGHT,
            350..=449 => Weight::NORMAL,
            450..=549 => Weight::MEDIUM,
            550..=649 => Weight::SEMIBOLD,
            650..=749 => Weight::BOLD,
            750..=849 => Weight::EXTRA_BOLD,
            _ => Weight::BLACK,
        })
}

fn rgba8_color(color: [f32; 4]) -> Color {
    let [r, g, b, a] = to_rgba8(color);
    Color::rgba(r, g, b, a)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_box_origin_keeps_left_edge_and_centers_vertically() {
        let bounds = LogicalRect::from_xywh(10.0, 20.0, 100.0, 40.0);
        assert_eq!(
            text_box_origin(bounds, TextVerticalAlignment::Top, 12.0),
            [10.0, 20.0]
        );
        assert_eq!(
            text_box_origin(bounds, TextVerticalAlignment::Center, 12.0),
            [10.0, 34.0]
        );
        assert_eq!(
            text_box_origin(bounds, TextVerticalAlignment::Bottom, 12.0),
            [10.0, 48.0]
        );
    }
}
