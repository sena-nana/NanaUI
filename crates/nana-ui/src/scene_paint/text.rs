use cosmic_text::{
    Align, Attrs, Buffer, Color, Family, FontSystem, Metrics, Shaping, SwashCache, Weight, Wrap,
};
use nana_ui_core::LineHeightSpec;
use nana_ui_runtime::{TextHorizontalAlignment, TextShaping, TextVerticalAlignment};
use nana_ui_scene::SceneTextSpan;
use std::collections::{HashMap, VecDeque};

use super::clip::LogicalRect;
use super::color::{to_rgba8, with_opacity};
use crate::PhysicalRect;

const SHAPE_CACHE_CAP: usize = 512;

pub(super) struct TextPipeline {
    font_system: FontSystem,
    #[allow(dead_code)]
    cache: cryoglyph::Cache,
    atlas: cryoglyph::TextAtlas,
    viewport: cryoglyph::Viewport,
    swash: SwashCache,
    renderers: Vec<cryoglyph::TextRenderer>,
    /// Shaped paragraphs reused across frames. Shaping is the dominant CPU
    /// cost of a text-heavy frame and identical text+style+box repeats on
    /// every repaint (scroll, hover, unrelated animations), so the shaped
    /// `Buffer` is cached and only glyph vertices are regenerated per frame.
    shape_cache: ShapeCache,
    frame_texts: usize,
    prev_frame_texts: usize,
}

#[derive(Default)]
struct ShapeCache {
    entries: HashMap<ShapeKey, Buffer>,
    order: VecDeque<ShapeKey>,
    hits: usize,
    misses: usize,
    evictions: usize,
}

impl ShapeCache {
    fn get(&mut self, key: &ShapeKey) -> Option<&Buffer> {
        if self.entries.contains_key(key) {
            self.hits += 1;
            self.entries.get(key)
        } else {
            self.misses += 1;
            None
        }
    }

    fn insert(&mut self, key: ShapeKey, buffer: Buffer) {
        if self.entries.contains_key(&key) {
            return;
        }
        while self.entries.len() >= SHAPE_CACHE_CAP {
            let Some(oldest) = self.order.pop_front() else {
                break;
            };
            self.entries.remove(&oldest);
            self.evictions += 1;
        }
        self.order.push_back(key.clone());
        self.entries.insert(key, buffer);
    }
}

/// Everything that determines the shaped output. Position is applied at draw
/// time via `TextArea` and plain-text color via `default_color`, so neither
/// is part of the key; rich spans bake their colors into shaping attrs and
/// therefore belong to it.
#[derive(Clone, PartialEq, Eq, Hash)]
struct ShapeKey {
    content: String,
    family: Option<String>,
    weight: Option<u16>,
    font_size_bits: u32,
    line_height_bits: u32,
    wrap: bool,
    ellipsis: bool,
    shaping: u8,
    width_bits: u32,
    height_bits: u32,
    align: u8,
    color: ColorKey,
}

#[derive(Clone, PartialEq, Eq, Hash)]
enum ColorKey {
    Plain,
    Rich { spans: Vec<(String, [u32; 4])> },
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
            shape_cache: ShapeCache::default(),
            frame_texts: 0,
            prev_frame_texts: 0,
        }
    }

    pub(super) fn begin_frame(&mut self, queue: &wgpu::Queue, physical_size: [u32; 2]) {
        self.prev_frame_texts = self.frame_texts;
        self.frame_texts = 0;
        // Renderer high-water decay: keep the GPU-side working set near the
        // last frame's text count instead of retaining a peak forever.
        let keep = self.prev_frame_texts + 8;
        if self.renderers.len() > keep {
            self.renderers.truncate(keep);
        }
        self.viewport.update(
            queue,
            cryoglyph::Resolution {
                width: physical_size[0].max(1),
                height: physical_size[1].max(1),
            },
        );
    }

    /// Shape-cache counters for tests: (hits, misses, evictions). None until
    /// consulted, mirroring the Runtime text cache contract.
    pub(super) fn shape_cache_stats(&self) -> (usize, usize, usize) {
        (
            self.shape_cache.hits,
            self.shape_cache.misses,
            self.shape_cache.evictions,
        )
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
        let painted = presentation_spans(content, spans, default_color, opacity);
        let rich = painted.len() > 1 || painted.first().is_some_and(|span| span.1 != default_color);
        let key = ShapeKey {
            content: content.to_owned(),
            family: family.map(str::to_owned),
            weight,
            font_size_bits: physical_size.to_bits(),
            line_height_bits: physical_line_height.to_bits(),
            wrap,
            ellipsis,
            shaping: match shaping {
                Shaping::Basic => 0,
                Shaping::Advanced => 1,
            },
            width_bits: physical_width.to_bits(),
            height_bits: physical_height.to_bits(),
            align: match horizontal {
                TextHorizontalAlignment::Start => 0,
                TextHorizontalAlignment::Center => 1,
                TextHorizontalAlignment::End => 2,
            },
            color: if rich {
                ColorKey::Rich {
                    spans: painted
                        .iter()
                        .map(|(text, color)| ((*text).to_owned(), color.map(f32::to_bits)))
                        .collect(),
                }
            } else {
                ColorKey::Plain
            },
        };
        if self.shape_cache.get(&key).is_none() {
            let mut buffer = Buffer::new(
                &mut self.font_system,
                Metrics::new(physical_size, physical_line_height),
            );
            buffer.set_size(Some(physical_width), Some(physical_height));
            buffer.set_wrap(if wrap { Wrap::Word } else { Wrap::None });
            buffer.set_ellipsize(if ellipsis {
                cosmic_text::Ellipsize::End(cosmic_text::EllipsizeHeightLimit::Height(
                    physical_height,
                ))
            } else {
                cosmic_text::Ellipsize::None
            });
            if rich {
                let rich_text = painted
                    .iter()
                    .map(|(text, color)| (*text, attrs.clone().color(rgba8_color(*color))))
                    .collect::<Vec<_>>();
                buffer.set_rich_text(rich_text, &attrs, shaping, align);
            } else {
                buffer.set_text(content, &attrs, shaping, align);
            }
            buffer.shape_until_scroll(&mut self.font_system, false);
            self.shape_cache.insert(key.clone(), buffer);
        }
        let buffer = self.shape_cache.entries.get(&key).expect("shaped above");
        let (_, laid_out_height) = measure(buffer);
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
        let index = self.frame_texts;
        self.frame_texts += 1;
        if self.renderers.len() <= index {
            self.renderers.push(cryoglyph::TextRenderer::new(
                &mut self.atlas,
                device,
                wgpu::MultisampleState::default(),
                None,
            ));
        }
        let area = cryoglyph::TextArea {
            text: buffer.layout_runs(),
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
                text: buffer.layout_runs(),
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
        gpu_work: Option<&crate::gpu_work::GpuWorkSink>,
    ) {
        let Some(renderer) = self.renderers.get(prepared.index) else {
            return;
        };
        pass.set_scissor_rect(scissor.x, scissor.y, scissor.width, scissor.height);
        let _ = renderer.render(&self.atlas, &self.viewport, pass);
        if let Some(work) = gpu_work {
            work.record_draw_batch();
            work.record_draw_call();
        }
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
