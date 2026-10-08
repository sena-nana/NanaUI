//! Semantic span colors of a [`SelectableRichText`](super::SelectableRichText).
//!
//! The spans still project as one text and one drawable. Each span with a
//! [`RichSpan::color`] becomes a byte range of the node's [`HighlightRequest`]
//! overlay, so the colors reach extraction on the `TextSpan` path syntax
//! highlighting takes and resolve against the installed palette there: a
//! theme change recolors them without projecting again. The presenter the
//! request names is never registered, so the base layer stays empty and only
//! the overlay paints.

use std::sync::Arc;

use super::RichSpan;
use crate::{HighlightRequest, TextSpan};

/// Presenter of the overlay. Not [`crate::HIGHLIGHT_PRESENTER`], so no syntax
/// highlighter ever runs over rich text.
pub(super) const SPAN_COLOR_PRESENTER: &str = "rich-span-color";

/// The overlay for `spans`: one byte range per colored span over their
/// concatenated text, neighbours of one role merged. `None` when no span has
/// a color, so the node keeps its own text color.
pub(super) fn color_request(spans: &[RichSpan]) -> Option<HighlightRequest> {
    let mut overlay: Vec<TextSpan> = Vec::new();
    let mut start = 0;
    for span in spans {
        let end = start + span.text.len();
        if let Some(color) = span.color.filter(|_| start < end) {
            match overlay.last_mut() {
                Some(last) if last.end == start && last.color == color => last.end = end,
                _ => overlay.push(TextSpan { start, end, color }),
            }
        }
        start = end;
    }
    (!overlay.is_empty())
        .then(|| HighlightRequest::new(SPAN_COLOR_PRESENTER, "").with_overlay(Arc::from(overlay)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{AppContext, DocumentId, SelectableRichText, SemanticColorRole, StandardVisual};

    fn document() -> DocumentId {
        DocumentId::new(1).unwrap()
    }

    /// A keyword, an uncolored gap, and two neighbouring keyword spans (one
    /// multi-byte): one text, one visual, and an overlay of byte ranges that
    /// extraction paints in the palette's keyword color.
    #[test]
    fn colored_spans_paint_through_one_text_overlay() {
        let mut context = AppContext::new();
        let text = context
            .create_component(
                document(),
                SelectableRichText::new([
                    RichSpan::plain("fn").color(SemanticColorRole::Keyword),
                    RichSpan::plain(" main"),
                    RichSpan::plain("你").color(SemanticColorRole::Keyword),
                    RichSpan::plain("好").color(SemanticColorRole::Keyword),
                    RichSpan::plain("").color(SemanticColorRole::Danger),
                    RichSpan::plain("\"s\"").color(SemanticColorRole::Success),
                ]),
            )
            .unwrap();
        let id = text.stable_id();
        let plain = "fn main你好\"s\"";
        let world = context.world();
        assert_eq!(world.text(id), Some(plain));
        assert!(matches!(
            world.standard_visual(id),
            Some(StandardVisual::SelectableRichText { text, .. }) if text.as_ref() == plain
        ));
        let request = world.highlight_request(id).expect("an overlay request");
        assert_eq!(request.presenter.as_ref(), SPAN_COLOR_PRESENTER);
        assert!(!world.has_presenter(SPAN_COLOR_PRESENTER));
        let keyword_end = "fn main你好".len();
        let expected = [
            TextSpan {
                start: 0,
                end: 2,
                color: SemanticColorRole::Keyword,
            },
            TextSpan {
                start: "fn main".len(),
                end: keyword_end,
                color: SemanticColorRole::Keyword,
            },
            TextSpan {
                start: keyword_end,
                end: plain.len(),
                color: SemanticColorRole::Success,
            },
        ];
        assert_eq!(request.overlay.as_deref(), Some(expected.as_slice()));

        context.resolve_presentations(&[id]).unwrap();
        let world = context.world();
        assert_eq!(
            world.text_presentation(id).expect("presentation").spans,
            expected
        );
        let extracted = world.extract_nodes(&[id]);
        assert_eq!(extracted.len(), 1);
        let palette = world.style_model();
        assert_eq!(
            extracted[0]
                .text_spans
                .iter()
                .map(|span| (span.start, span.end, span.color))
                .collect::<Vec<_>>(),
            expected
                .iter()
                .map(|span| (
                    span.start,
                    span.end,
                    palette.color(span.color).as_rgba_array()
                ))
                .collect::<Vec<_>>()
        );
    }

    /// Spans without a color ask for nothing, and taking every color away
    /// drops the overlay and the painted ranges with it.
    #[test]
    fn uncolored_spans_keep_the_node_text_color() {
        let mut context = AppContext::new();
        let text = context
            .create_component(
                document(),
                SelectableRichText::new([RichSpan::plain("let").color(SemanticColorRole::Keyword)]),
            )
            .unwrap();
        let id = text.stable_id();
        context.resolve_presentations(&[id]).unwrap();
        assert_eq!(context.world().extract_nodes(&[id])[0].text_spans.len(), 1);

        context
            .update_component(text, |text, _| {
                *text = SelectableRichText::new([RichSpan::plain("let")]);
            })
            .unwrap();
        context.resolve_presentations(&[id]).unwrap();
        let world = context.world();
        assert_eq!(world.highlight_request(id), None);
        assert!(world.text_presentation(id).is_none());
        assert!(world.extract_nodes(&[id])[0].text_spans.is_empty());
        assert_eq!(color_request(&[RichSpan::plain("")]), None);
    }
}
