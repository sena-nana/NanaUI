use iced::advanced::graphics::text::cosmic_text::{Affinity, Buffer, Cursor as CosmicCursor};
use iced::advanced::text::{
    Alignment, Ellipsis, LineHeight, Paragraph, Renderer as TextRenderer, Shaping, Text, Wrapping,
};
use iced::{Pixels, Size, alignment, font};
use nana_ui_runtime::{
    ComputedStyle, LayoutBox, StableNodeId, TextContent, TextMetrics, TextShapeConstraints,
    TextShaper, TextShaping,
};
use unicode_segmentation::UnicodeSegmentation;

use crate::ui_font;

type RendererParagraph = <iced::Renderer as TextRenderer>::Paragraph;

/// Compatibility text backend used by canonical Runtime frames.
///
/// Shaping uses the same Iced/Cryoglyph path as visible NanaUI text while the
/// retained metrics remain backend-neutral Runtime data.
#[derive(Debug, Default, Clone, Copy)]
pub struct IcedTextShaper;

impl TextShaper for IcedTextShaper {
    fn shape(
        &mut self,
        _id: StableNodeId,
        text: &TextContent,
        style: &ComputedStyle,
        constraints: TextShapeConstraints,
    ) -> TextMetrics {
        let paragraph = paragraph(text, style, constraints);
        let bounds = paragraph.min_bounds();
        let tracking = style.letter_spacing * text.value.chars().count().saturating_sub(1) as f32;
        TextMetrics {
            width: (bounds.width + tracking).max(0.0),
            height: bounds.height,
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
        let paragraph = paragraph(
            text,
            style,
            TextShapeConstraints {
                shaping: TextShaping::Advanced,
                ..TextShapeConstraints::default()
            },
        );
        let graphemes = text.value[..byte_offset].graphemes(true).count();
        paragraph
            .grapheme_position(0, graphemes)
            .map_or(0.0, |position| {
                position.x + style.letter_spacing * graphemes.saturating_sub(1) as f32
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
        let paragraph = paragraph(text, style, constraints);
        let Some(cursor) = cosmic_cursor(paragraph.buffer(), byte_offset, Affinity::After) else {
            return (0.0, 0.0, 0.0);
        };
        let mut position = None;
        for run in paragraph.buffer().layout_runs() {
            if let Some(x) = run.cursor_position(&cursor) {
                position = Some((x, run.line_top, run.line_height));
            }
        }
        if let Some(position) = position {
            return position;
        }
        paragraph
            .buffer()
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
        let paragraph = paragraph(text, style, constraints);
        let Some(start) = cosmic_cursor(paragraph.buffer(), start, Affinity::After) else {
            return Vec::new();
        };
        let Some(end) = cosmic_cursor(paragraph.buffer(), end, Affinity::Before) else {
            return Vec::new();
        };
        let mut highlights = Vec::new();
        for run in paragraph.buffer().layout_runs() {
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

fn is_grapheme_boundary(value: &str, offset: usize) -> bool {
    offset == value.len()
        || value
            .grapheme_indices(true)
            .any(|(boundary, _)| boundary == offset)
}

fn cosmic_cursor(buffer: &Buffer, byte_offset: usize, affinity: Affinity) -> Option<CosmicCursor> {
    let mut base = 0;
    for (line, content) in buffer.lines.iter().enumerate() {
        let line_end = base + content.text().len();
        if byte_offset <= line_end {
            return Some(CosmicCursor::new_with_affinity(
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

fn resolved_line_height(style: &ComputedStyle) -> f32 {
    match style.line_height {
        Some(nana_ui_core::LineHeightSpec::Absolute(value)) => value.max(0.0),
        Some(nana_ui_core::LineHeightSpec::Relative(value)) => style.font_size * value.max(0.0),
        None => style.font_size * 1.2,
    }
}

fn paragraph(
    text: &TextContent,
    style: &ComputedStyle,
    constraints: TextShapeConstraints,
) -> RendererParagraph {
    let font = resolve_font(style.font_family.as_deref(), font_weight(style.font_weight));
    RendererParagraph::with_text(Text {
        content: text.value.as_str(),
        bounds: Size::new(
            constraints.max_width.unwrap_or(f32::INFINITY),
            constraints.max_height.unwrap_or(f32::INFINITY),
        ),
        size: Pixels(style.font_size),
        line_height: style.line_height.map(line_height).unwrap_or_default(),
        font,
        align_x: Alignment::Default,
        align_y: alignment::Vertical::Top,
        shaping: match constraints.shaping {
            TextShaping::Auto => Shaping::Auto,
            TextShaping::Advanced => Shaping::Advanced,
        },
        wrapping: if constraints.wrap {
            Wrapping::Word
        } else {
            Wrapping::None
        },
        ellipsis: if constraints.ellipsis {
            Ellipsis::End
        } else {
            Ellipsis::None
        },
        hint_factor: None,
    })
}

fn line_height(spec: nana_ui_core::LineHeightSpec) -> LineHeight {
    match spec {
        nana_ui_core::LineHeightSpec::Relative(value) => LineHeight::Relative(value.max(0.0)),
        nana_ui_core::LineHeightSpec::Absolute(value) => {
            LineHeight::Absolute(Pixels(value.max(0.0)))
        }
    }
}

fn font_weight(weight: Option<u16>) -> font::Weight {
    match weight.unwrap_or(400) {
        0..=199 => font::Weight::Thin,
        200..=299 => font::Weight::ExtraLight,
        300..=349 => font::Weight::Light,
        350..=449 => font::Weight::Normal,
        450..=549 => font::Weight::Medium,
        550..=649 => font::Weight::Semibold,
        650..=749 => font::Weight::Bold,
        750..=849 => font::Weight::ExtraBold,
        _ => font::Weight::Black,
    }
}

fn resolve_font(family: Option<&str>, weight: font::Weight) -> iced::Font {
    match family
        .map(str::trim)
        .map(str::to_ascii_lowercase)
        .as_deref()
    {
        Some("monospace") | Some("ui-monospace") => iced::Font {
            weight,
            ..iced::Font::MONOSPACE
        },
        _ => ui_font(weight),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nana_ui_runtime::{
        AccessibilityAction, AccessibilityActionRequest, ComponentGeometry, DocumentId,
        LayoutViewport, TextArea, TextInput, TextSelection,
    };
    use nana_ui_scene::RuntimeDocument;

    #[test]
    fn shapes_cjk_with_the_visible_runtime_backend() {
        let metrics = IcedTextShaper.shape(
            StableNodeId::new(1).unwrap(),
            &TextContent {
                value: "输入法".into(),
            },
            &ComputedStyle::default(),
            TextShapeConstraints::default(),
        );
        assert!(metrics.width > 0.0);
        assert!(metrics.height > 0.0);
    }

    #[test]
    fn cursor_offsets_follow_shaped_graphemes_instead_of_utf8_width_guesses() {
        let text = TextContent {
            value: "A👩‍💻界".into(),
        };
        let style = ComputedStyle::default();
        let after_ascii = IcedTextShaper.horizontal_offset(
            StableNodeId::new(1).unwrap(),
            &text,
            "A".len(),
            &style,
        );
        let after_emoji = IcedTextShaper.horizontal_offset(
            StableNodeId::new(1).unwrap(),
            &text,
            "A👩‍💻".len(),
            &style,
        );
        let at_end = IcedTextShaper.horizontal_offset(
            StableNodeId::new(1).unwrap(),
            &text,
            text.value.len(),
            &style,
        );
        assert!(after_ascii > 0.0);
        assert!(after_emoji > after_ascii);
        assert!(at_end > after_emoji);
    }

    #[test]
    fn wrapped_cjk_and_emoji_share_iced_caret_and_selection_lines() {
        let id = StableNodeId::new(1).unwrap();
        let text = TextContent {
            value: "甲乙👩‍💻丙丁戊".into(),
        };
        let style = ComputedStyle {
            font_size: 16.0,
            line_height: Some(nana_ui_core::LineHeightSpec::Absolute(20.0)),
            ..ComputedStyle::default()
        };
        let two_cjk = IcedTextShaper.shape(
            id,
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

        let (_, caret_y, line_height) =
            IcedTextShaper.text_position(id, &text, "甲乙👩‍💻".len(), &style, wrapped);
        let highlights = IcedTextShaper.text_highlights(
            id,
            &text,
            ("甲".len(), "甲乙👩‍💻丙丁".len()),
            &style,
            wrapped,
        );

        assert_eq!(line_height, 20.0);
        assert!(caret_y >= line_height);
        assert!(highlights.len() >= 2);
        assert!(highlights.windows(2).all(|lines| lines[0].y < lines[1].y));
        assert!(highlights.iter().all(|line| line.width > 0.0));

        let (_, single_line_y, _) = IcedTextShaper.text_position(
            id,
            &text,
            text.value.len(),
            &style,
            TextShapeConstraints {
                shaping: TextShaping::Advanced,
                ..TextShapeConstraints::default()
            },
        );
        assert_eq!(single_line_y, 0.0);
    }

    #[test]
    fn selection_does_not_leak_into_unselected_explicit_lines() {
        let id = StableNodeId::new(1).unwrap();
        let text = TextContent {
            value: "First line\nSecond line\nThird line".into(),
        };
        let highlights = IcedTextShaper.text_highlights(
            id,
            &text,
            (0, "First ".len()),
            &ComputedStyle::default(),
            TextShapeConstraints {
                max_width: Some(320.0),
                wrap: true,
                shaping: TextShaping::Advanced,
                ..TextShapeConstraints::default()
            },
        );

        assert_eq!(highlights.len(), 1);
        assert_eq!(highlights[0].y, 0.0);
        assert!(highlights[0].width > 0.0);
    }

    #[test]
    fn bidi_selection_uses_all_disjoint_cosmic_highlight_spans() {
        let id = StableNodeId::new(1).unwrap();
        let text = TextContent {
            value: "abc אבג xyz".into(),
        };
        let highlights = IcedTextShaper.text_highlights(
            id,
            &text,
            ("ab".len(), "abc אב".len()),
            &ComputedStyle::default(),
            TextShapeConstraints {
                shaping: TextShaping::Advanced,
                ..TextShapeConstraints::default()
            },
        );

        assert!(highlights.len() >= 2);
        assert!(highlights.iter().all(|span| span.width > 0.0));
        assert!(highlights.iter().all(|span| span.y == 0.0));
    }

    #[test]
    fn crlf_is_one_line_ending_and_emoji_is_one_caret_step() {
        let id = StableNodeId::new(1).unwrap();
        let text = TextContent {
            value: "甲\r\n👩‍💻乙".into(),
        };
        let style = ComputedStyle {
            line_height: Some(nana_ui_core::LineHeightSpec::Absolute(20.0)),
            ..ComputedStyle::default()
        };
        let constraints = TextShapeConstraints {
            shaping: TextShaping::Advanced,
            ..TextShapeConstraints::default()
        };
        let second_line = "甲\r\n".len();
        let after_emoji = second_line + "👩‍💻".len();
        let cursor = cosmic_cursor(
            paragraph(&text, &style, constraints).buffer(),
            second_line,
            Affinity::After,
        )
        .unwrap();
        assert_eq!((cursor.line, cursor.index), (1, 0));

        let (line_start_x, line_start_y, _) =
            IcedTextShaper.text_position(id, &text, second_line, &style, constraints);
        let (after_emoji_x, after_emoji_y, _) =
            IcedTextShaper.text_position(id, &text, after_emoji, &style, constraints);
        assert_eq!(line_start_x, 0.0);
        assert_eq!(line_start_y, 20.0);
        assert_eq!(after_emoji_y, line_start_y);
        assert!(after_emoji_x > line_start_x);
    }

    #[test]
    fn runtime_single_line_stays_unwrapped_and_reveals_the_caret_horizontally() {
        let document_id = DocumentId::new(1).unwrap();
        let mut document = RuntimeDocument::new(document_id);
        let value = "release/a-very-long-branch-name";
        let input = document
            .context_mut()
            .create_component(document_id, TextInput::new(value))
            .unwrap();
        document
            .context_mut()
            .focus_node(document_id, input.stable_id())
            .unwrap();
        document
            .context_mut()
            .apply_accessibility_action(
                document_id,
                AccessibilityActionRequest {
                    target: input.stable_id(),
                    action: AccessibilityAction::SetSelection(TextSelection::caret(value.len())),
                },
            )
            .unwrap();
        document
            .flush(LayoutViewport::new(120.0, 80.0), &mut IcedTextShaper)
            .unwrap();

        let layout = document
            .context()
            .world()
            .layout_box(input.stable_id())
            .unwrap();
        let geometry = document
            .context()
            .world()
            .component_geometry(input.stable_id())
            .unwrap();
        let ComponentGeometry::TextInput {
            multiline,
            text,
            caret: Some(caret),
            ..
        } = geometry
        else {
            panic!("focused input must produce text and caret geometry");
        };
        assert!(!multiline);
        assert!(text.bounds.width > layout.width);
        assert!(text.bounds.x < layout.x);
        assert!(caret.x >= layout.x && caret.x <= layout.x + layout.width);
        assert!(caret.y >= layout.y && caret.y + caret.height <= layout.y + layout.height);
    }

    #[test]
    fn runtime_textarea_wraps_full_content_and_reveals_the_last_visual_line() {
        let document_id = DocumentId::new(1).unwrap();
        let mut document = RuntimeDocument::new(document_id);
        let value = "甲乙👩‍💻丙丁戊己庚辛壬癸甲乙👩‍💻丙丁戊己庚辛壬癸甲乙👩‍💻丙丁戊己庚辛壬癸";
        let input = document
            .context_mut()
            .create_component(document_id, TextArea::new(value).height(96.0))
            .unwrap();
        document
            .context_mut()
            .focus_node(document_id, input.stable_id())
            .unwrap();
        document
            .context_mut()
            .apply_accessibility_action(
                document_id,
                AccessibilityActionRequest {
                    target: input.stable_id(),
                    action: AccessibilityAction::SetSelection(TextSelection::caret(value.len())),
                },
            )
            .unwrap();
        document
            .flush(LayoutViewport::new(84.0, 100.0), &mut IcedTextShaper)
            .unwrap();

        let layout = document
            .context()
            .world()
            .layout_box(input.stable_id())
            .unwrap();
        let geometry = document
            .context()
            .world()
            .component_geometry(input.stable_id())
            .unwrap();
        let ComponentGeometry::TextInput {
            multiline,
            text,
            caret: Some(caret),
            ..
        } = geometry
        else {
            panic!("focused textarea must produce text and caret geometry");
        };
        assert!(multiline);
        assert_eq!(layout.height, 96.0);
        assert!(text.bounds.height > layout.height);
        assert!(text.bounds.y < layout.y);
        assert!(caret.y >= layout.y && caret.y + caret.height <= layout.y + layout.height);
    }
}
