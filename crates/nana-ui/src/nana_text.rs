//! Nana-owned cosmic-text shaper. Layout metrics stay on Runtime.

use cosmic_text::{
    Affinity, Attrs, Buffer, Cursor, Ellipsize, EllipsizeHeightLimit, Family, FontSystem, Metrics,
    Shaping, Weight, Wrap,
};
use nana_ui_core::LineHeightSpec;
use nana_ui_runtime::{
    ComputedStyle, GlyphCache, LayoutBox, StableNodeId, TextContent, TextMetrics,
    TextShapeConstraints, TextShaper, TextShaping,
};
use unicode_segmentation::UnicodeSegmentation;

/// Product text shaper for Runtime flush on the Nana WGPU host path.
#[derive(Debug)]
pub struct NanaTextShaper {
    font_system: FontSystem,
}

impl Default for NanaTextShaper {
    fn default() -> Self {
        Self {
            font_system: nana_font_system(),
        }
    }
}

pub(crate) fn nana_font_system() -> FontSystem {
    #[cfg(not(feature = "bundled-fonts"))]
    {
        FontSystem::new()
    }
    #[cfg(feature = "bundled-fonts")]
    {
        let mut font_system = FontSystem::new();
        for source in crate::ui_font_sources() {
            let _ = font_system
                .db_mut()
                .load_font_source(cosmic_text::fontdb::Source::Binary(std::sync::Arc::new(
                    source,
                )));
        }
        font_system.db_mut().set_sans_serif_family("Noto Sans SC");
        font_system
    }
}

impl TextShaper for NanaTextShaper {
    fn shape(
        &mut self,
        _id: StableNodeId,
        text: &TextContent,
        style: &ComputedStyle,
        constraints: TextShapeConstraints,
    ) -> TextMetrics {
        let buffer = self.shape_buffer(&text.value, style, constraints);
        let (width, height, _) = measure(&buffer);
        let tracking = style.letter_spacing * text.value.chars().count().saturating_sub(1) as f32;
        TextMetrics {
            width: (width + tracking).max(0.0),
            height,
        }
    }

    fn shape_cached(
        &mut self,
        _id: StableNodeId,
        text: &TextContent,
        style: &ComputedStyle,
        constraints: TextShapeConstraints,
        glyphs: &mut GlyphCache,
    ) -> TextMetrics {
        if !constraints.wrap
            && !constraints.ellipsis
            && let Some(ch) = single_char(&text.value)
            && let Some(advance) = glyphs.peek(ch, style)
        {
            let _ = glyphs.lookup(ch, style);
            return metrics_from_advance(advance, style, text.value.chars().count());
        }
        let buffer = self.shape_buffer(&text.value, style, constraints);
        record_shaped_glyphs(&buffer, style, glyphs);
        let (width, height, _) = measure(&buffer);
        let tracking = style.letter_spacing * text.value.chars().count().saturating_sub(1) as f32;
        TextMetrics {
            width: (width + tracking).max(0.0),
            height,
        }
    }

    fn horizontal_offset(
        &mut self,
        _id: StableNodeId,
        text: &TextContent,
        byte_offset: usize,
        style: &ComputedStyle,
    ) -> f32 {
        if byte_offset > text.value.len()
            || !text.value.is_char_boundary(byte_offset)
            || !is_grapheme_boundary(&text.value, byte_offset)
        {
            return 0.0;
        }
        let buffer = self.shape_buffer(
            &text.value,
            style,
            TextShapeConstraints {
                shaping: TextShaping::Advanced,
                ..TextShapeConstraints::default()
            },
        );
        let graphemes = text.value[..byte_offset].graphemes(true).count();
        grapheme_x(&buffer, 0, graphemes).map_or(0.0, |x| {
            x + style.letter_spacing * graphemes.saturating_sub(1) as f32
        })
    }

    fn text_position(
        &mut self,
        _id: StableNodeId,
        text: &TextContent,
        byte_offset: usize,
        style: &ComputedStyle,
        constraints: TextShapeConstraints,
    ) -> (f32, f32, f32) {
        if byte_offset > text.value.len()
            || !text.value.is_char_boundary(byte_offset)
            || !is_grapheme_boundary(&text.value, byte_offset)
        {
            return (0.0, 0.0, 0.0);
        }
        let buffer = self.shape_buffer(&text.value, style, constraints);
        let Some(cursor) = cosmic_cursor(&buffer, byte_offset, Affinity::After) else {
            return (0.0, 0.0, 0.0);
        };
        let mut position = None;
        for run in buffer.layout_runs() {
            if let Some(x) = run.cursor_position(&cursor) {
                position = Some((x, run.line_top, run.line_height));
            }
        }
        if let Some(position) = position {
            return position;
        }
        buffer
            .layout_runs()
            .find(|run| run.line_i == cursor.line && run.glyphs.is_empty())
            .map_or((0.0, 0.0, resolved_line_height(style)), |run| {
                (0.0, run.line_top, run.line_height)
            })
    }

    fn text_highlights(
        &mut self,
        _id: StableNodeId,
        text: &TextContent,
        selection: (usize, usize),
        style: &ComputedStyle,
        constraints: TextShapeConstraints,
    ) -> Vec<LayoutBox> {
        let (start, end) = selection;
        if start >= end
            || end > text.value.len()
            || !text.value.is_char_boundary(start)
            || !text.value.is_char_boundary(end)
            || !is_grapheme_boundary(&text.value, start)
            || !is_grapheme_boundary(&text.value, end)
        {
            return Vec::new();
        }
        let buffer = self.shape_buffer(&text.value, style, constraints);
        let Some(start) = cosmic_cursor(&buffer, start, Affinity::After) else {
            return Vec::new();
        };
        let Some(end) = cosmic_cursor(&buffer, end, Affinity::Before) else {
            return Vec::new();
        };
        let mut highlights = Vec::new();
        for run in buffer.layout_runs() {
            if run.line_i < start.line || run.line_i > end.line {
                continue;
            }
            for (x, width) in run.highlight(start, end) {
                highlights.push(LayoutBox {
                    x,
                    y: run.line_top,
                    width,
                    height: run.line_height,
                });
            }
        }
        highlights
    }
}

impl NanaTextShaper {
    fn shape_buffer(
        &mut self,
        text: &str,
        style: &ComputedStyle,
        constraints: TextShapeConstraints,
    ) -> Buffer {
        let font_size = style.font_size.max(f32::MIN_POSITIVE);
        let line_height = resolved_line_height(style).max(f32::MIN_POSITIVE);
        let mut buffer = Buffer::new(&mut self.font_system, Metrics::new(font_size, line_height));
        buffer.set_size(
            Some(constraints.max_width.unwrap_or(f32::INFINITY)),
            Some(constraints.max_height.unwrap_or(f32::INFINITY)),
        );
        buffer.set_wrap(if constraints.wrap {
            Wrap::Word
        } else {
            Wrap::None
        });
        buffer.set_ellipsize(if constraints.ellipsis {
            Ellipsize::End(EllipsizeHeightLimit::Height(
                constraints.max_height.unwrap_or(f32::INFINITY),
            ))
        } else {
            Ellipsize::None
        });
        let attrs = text_attrs(style);
        let shaping = match constraints.shaping {
            TextShaping::Auto | TextShaping::Advanced => Shaping::Advanced,
        };
        buffer.set_text(text, &attrs, shaping, None);
        buffer.shape_until_scroll(&mut self.font_system, false);

        let (min_width, min_height, has_rtl) = measure(&buffer);
        if has_rtl {
            buffer.set_size(Some(min_width), Some(min_height));
            buffer.shape_until_scroll(&mut self.font_system, false);
        }
        buffer
    }
}

fn text_attrs(style: &ComputedStyle) -> Attrs<'_> {
    Attrs::new()
        .family(resolve_family(style.font_family.as_deref()))
        .weight(font_weight(style.font_weight))
}

fn resolve_family(family: Option<&str>) -> Family<'_> {
    let trimmed = family.map(str::trim).unwrap_or("");
    if trimmed.is_empty() {
        return Family::SansSerif;
    }
    let lowered = trimmed.to_ascii_lowercase();
    if matches!(lowered.as_str(), "sans-serif" | "system-ui") {
        Family::SansSerif
    } else if lowered.contains("mono") {
        Family::Monospace
    } else {
        Family::Name(trimmed)
    }
}

fn font_weight(weight: Option<u16>) -> Weight {
    match weight.unwrap_or(400) {
        0..=199 => Weight::THIN,
        200..=299 => Weight::EXTRA_LIGHT,
        300..=349 => Weight::LIGHT,
        350..=449 => Weight::NORMAL,
        450..=549 => Weight::MEDIUM,
        550..=649 => Weight::SEMIBOLD,
        650..=749 => Weight::BOLD,
        750..=849 => Weight::EXTRA_BOLD,
        _ => Weight::BLACK,
    }
}

fn single_char(text: &str) -> Option<char> {
    let mut chars = text.chars();
    match (chars.next(), chars.next()) {
        (Some(ch), None) => Some(ch),
        _ => None,
    }
}

fn metrics_from_advance(advance: f32, style: &ComputedStyle, char_count: usize) -> TextMetrics {
    let tracking = style.letter_spacing * char_count.saturating_sub(1) as f32;
    TextMetrics {
        width: (advance + tracking).max(0.0),
        height: resolved_line_height(style),
    }
}

fn record_shaped_glyphs(buffer: &Buffer, style: &ComputedStyle, glyphs: &mut GlyphCache) {
    for run in buffer.layout_runs() {
        for glyph in run.glyphs {
            let cluster = &run.text[glyph.start..glyph.end];
            let mut chars = cluster.chars();
            let Some(ch) = chars.next() else {
                continue;
            };
            if chars.next().is_some() {
                continue;
            }
            if glyphs.lookup(ch, style).is_none() {
                glyphs.insert(ch, style, glyph.w);
            }
        }
    }
}

fn resolved_line_height(style: &ComputedStyle) -> f32 {
    match style.line_height {
        Some(LineHeightSpec::Absolute(value)) => value.max(0.0),
        Some(LineHeightSpec::Relative(value)) => style.font_size * value.max(0.0),
        None => style.font_size * 1.2,
    }
}

fn measure(buffer: &Buffer) -> (f32, f32, bool) {
    buffer
        .layout_runs()
        .fold((0.0, 0.0, false), |(width, height, has_rtl), run| {
            (
                run.line_w.max(width),
                height + run.line_height,
                has_rtl || run.rtl,
            )
        })
}

fn grapheme_x(buffer: &Buffer, line: usize, index: usize) -> Option<f32> {
    let run = buffer.layout_runs().nth(line)?;
    let mut last_start = None;
    let mut last_grapheme_count = 0;
    let mut graphemes_seen = 0;

    let glyph = run
        .glyphs
        .iter()
        .find(|glyph| {
            if Some(glyph.start) != last_start {
                last_grapheme_count = run.text[glyph.start..glyph.end].graphemes(false).count();
                last_start = Some(glyph.start);
                graphemes_seen += last_grapheme_count;
            }
            graphemes_seen >= index
        })
        .or_else(|| run.glyphs.last())?;

    let advance = if index == 0 {
        0.0
    } else {
        glyph.w
            * (1.0
                - graphemes_seen.saturating_sub(index) as f32 / last_grapheme_count.max(1) as f32)
    };

    Some(glyph.x + glyph.x_offset * glyph.font_size + advance)
}

fn is_grapheme_boundary(value: &str, offset: usize) -> bool {
    offset == value.len()
        || value
            .grapheme_indices(true)
            .any(|(boundary, _)| boundary == offset)
}

fn cosmic_cursor(buffer: &Buffer, byte_offset: usize, affinity: Affinity) -> Option<Cursor> {
    let mut base = 0;
    for (line, content) in buffer.lines.iter().enumerate() {
        let line_end = base + content.text().len();
        if byte_offset <= line_end {
            return Some(Cursor::new_with_affinity(
                line,
                byte_offset - base,
                affinity,
            ));
        }
        base = line_end + content.ending().as_str().len();
        if byte_offset < base {
            return None;
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn node() -> StableNodeId {
        StableNodeId::new(1).unwrap()
    }

    fn assert_positive_finite(metrics: TextMetrics) {
        assert!(metrics.width.is_finite() && metrics.width > 0.0);
        assert!(metrics.height.is_finite() && metrics.height > 0.0);
    }

    #[test]
    fn shapes_ascii_within_a_finite_max_width() {
        let metrics = NanaTextShaper::default().shape(
            node(),
            &TextContent {
                value: "Hello, Nana".into(),
            },
            &ComputedStyle::default(),
            TextShapeConstraints {
                max_width: Some(240.0),
                wrap: true,
                shaping: TextShaping::Advanced,
                ..TextShapeConstraints::default()
            },
        );
        assert_positive_finite(metrics);
    }

    #[test]
    fn shapes_cjk_weekday_with_nonzero_width() {
        let metrics = NanaTextShaper::default().shape(
            node(),
            &TextContent {
                value: "周一".into(),
            },
            &ComputedStyle::default(),
            TextShapeConstraints::default(),
        );
        assert!(metrics.width.is_finite() && metrics.width > 0.0);
        assert!(metrics.height.is_finite() && metrics.height > 0.0);
    }

    #[test]
    fn invalid_byte_offsets_return_zero_and_empty_highlights() {
        let mut shaper = NanaTextShaper::default();
        let text = TextContent {
            value: "周一👩‍💻".into(),
        };
        let style = ComputedStyle::default();
        let constraints = TextShapeConstraints::default();
        let mid_char = 1;
        let past_end = text.value.len() + 4;
        let mid_emoji = "周一".len() + 1;

        assert_eq!(
            shaper.horizontal_offset(node(), &text, mid_char, &style),
            0.0
        );
        assert_eq!(
            shaper.horizontal_offset(node(), &text, past_end, &style),
            0.0
        );
        assert_eq!(
            shaper.horizontal_offset(node(), &text, mid_emoji, &style),
            0.0
        );
        assert_eq!(
            shaper.text_position(node(), &text, mid_char, &style, constraints),
            (0.0, 0.0, 0.0)
        );
        assert_eq!(
            shaper.text_position(node(), &text, past_end, &style, constraints),
            (0.0, 0.0, 0.0)
        );
        assert!(
            shaper
                .text_highlights(
                    node(),
                    &text,
                    (mid_char, text.value.len()),
                    &style,
                    constraints
                )
                .is_empty()
        );
        assert!(
            shaper
                .text_highlights(node(), &text, (0, past_end), &style, constraints)
                .is_empty()
        );
        assert!(
            shaper
                .text_highlights(node(), &text, (text.value.len(), 0), &style, constraints)
                .is_empty()
        );
    }

    #[test]
    fn highlight_rects_are_ordered_and_finite() {
        let mut shaper = NanaTextShaper::default();
        let style = ComputedStyle {
            font_size: 16.0,
            line_height: Some(LineHeightSpec::Absolute(20.0)),
            ..ComputedStyle::default()
        };
        let two_cjk = shaper.shape(
            node(),
            &TextContent {
                value: "甲乙".into(),
            },
            &style,
            TextShapeConstraints {
                shaping: TextShaping::Advanced,
                ..TextShapeConstraints::default()
            },
        );
        let wrapped = TextShapeConstraints {
            max_width: Some(two_cjk.width + 1.0),
            wrap: true,
            shaping: TextShaping::Advanced,
            ..TextShapeConstraints::default()
        };
        let text = TextContent {
            value: "甲乙👩‍💻丙丁戊".into(),
        };
        let highlights = shaper.text_highlights(
            node(),
            &text,
            ("甲".len(), "甲乙👩‍💻丙丁".len()),
            &style,
            wrapped,
        );

        assert!(highlights.len() >= 2);
        assert!(highlights.windows(2).all(|lines| lines[0].y < lines[1].y));
        assert!(highlights.iter().all(|line| {
            line.width.is_finite()
                && line.height.is_finite()
                && line.x.is_finite()
                && line.y.is_finite()
                && line.width > 0.0
                && line.height > 0.0
        }));

        let unknown = shaper.shape(
            node(),
            &TextContent {
                value: "Hello".into(),
            },
            &ComputedStyle {
                font_family: Some("DefinitelyNotARealFamily".into()),
                ..ComputedStyle::default()
            },
            TextShapeConstraints::default(),
        );
        assert_positive_finite(unknown);
    }

    #[test]
    fn glyph_cache_stores_advances_and_world_counts_miss_then_hit() {
        let mut shaper = NanaTextShaper::default();
        let mut glyphs = GlyphCache::default();
        let style = ComputedStyle {
            font_size: 16.0,
            ..ComputedStyle::default()
        };
        let constraints = TextShapeConstraints {
            shaping: TextShaping::Advanced,
            ..TextShapeConstraints::default()
        };
        let first = shaper.shape_cached(
            node(),
            &TextContent { value: "ab".into() },
            &style,
            constraints,
            &mut glyphs,
        );
        assert_positive_finite(first);
        let advance_a = glyphs.peek('a', &style).expect("shaped 'a' must be cached");
        let advance_b = glyphs.peek('b', &style).expect("shaped 'b' must be cached");
        assert!(advance_a > 0.0 && advance_a.is_finite());
        assert!(advance_b > 0.0 && advance_b.is_finite());

        let reused = shaper.shape_cached(
            node(),
            &TextContent { value: "a".into() },
            &style,
            constraints,
            &mut glyphs,
        );
        assert!((reused.width - advance_a).abs() < 0.01);
        assert!(reused.height.is_finite() && reused.height > 0.0);

        let mut world = nana_ui_runtime::UiWorld::new();
        let document = nana_ui_runtime::DocumentId::new(1).unwrap();
        let id = nana_ui_runtime::StableNodeId::new(1).unwrap();
        let mut queue = nana_ui_runtime::MutationQueue::new();
        queue.create(id, document, nana_ui_runtime::NodeKind::Text);
        queue.set_text(id, TextContent { value: "ab".into() });
        world.commit(queue).unwrap();
        let work = world.take_system_work();
        world.resolve_styles(&work.style).unwrap();
        let mut world_shaper = NanaTextShaper::default();
        world.shape_text(&work.text, &mut world_shaper).unwrap();
        let missed = world.last_work_counters();
        assert_eq!(missed.glyph_cache_misses, Some(2));
        assert_eq!(missed.glyph_cache_hits, Some(0));

        let mut patch = nana_ui_runtime::MutationQueue::new();
        patch.set_text(id, TextContent { value: "ba".into() });
        world.commit(patch).unwrap();
        let reused_work = world.take_system_work();
        world.resolve_styles(&reused_work.style).unwrap();
        world
            .shape_text(&reused_work.text, &mut world_shaper)
            .unwrap();
        let hit = world.last_work_counters();
        assert_eq!(hit.glyph_cache_hits, Some(2));
        assert_eq!(hit.glyph_cache_misses, Some(0));
    }
}
