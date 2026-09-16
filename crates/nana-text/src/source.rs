//! The authored text plus its style spans and a revision that cannot be dodged.
//!
//! The fields are private on purpose: the only way to change the bytes is a
//! method that bumps [`TextRevision`], which is what makes the revision worth
//! caching on.

use crate::id::TextRevision;
use crate::style::TextStyle;
use serde::{Deserialize, Serialize};
use std::ops::Range;

/// An IME composition marker.
///
/// Orthogonal to styling: a preedit run is a *state* of the text, not a font
/// choice, so it does not live inside [`TextStyle`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum CompositionSegment {
    /// Under composition.
    Preedit,
    /// The segment the IME currently targets within the composition.
    PreeditTarget,
}

/// One styled byte range of a [`TextSource`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TextSpan {
    /// Byte range into [`TextSource::text`]. Must land on char boundaries.
    pub range: Range<usize>,
    /// A full style, not a sparse override. A sparse override layer is a later
    /// decision, deliberately not half-built here.
    pub style: TextStyle,
    #[serde(default)]
    pub composition: Option<CompositionSegment>,
}

/// Authored text, its spans, and the revision they are at.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TextSource {
    text: String,
    spans: Vec<TextSpan>,
    revision: TextRevision,
}

impl Default for TextSource {
    fn default() -> Self {
        Self::new(String::new())
    }
}

impl TextSource {
    pub fn new(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            spans: Vec::new(),
            revision: TextRevision::INITIAL,
        }
    }

    pub fn text(&self) -> &str {
        &self.text
    }

    /// Sorted, non-overlapping. Empty means the whole string uses the caller's
    /// base style.
    pub fn spans(&self) -> &[TextSpan] {
        &self.spans
    }

    pub fn revision(&self) -> TextRevision {
        self.revision
    }

    pub fn is_empty(&self) -> bool {
        self.text.is_empty()
    }

    /// Replaces a byte range and shifts every span that sat after it.
    ///
    /// Spans overlapping the edit are dropped rather than guessed at: a wrong
    /// surviving span is harder to notice than a missing one.
    pub fn replace_range(&mut self, range: Range<usize>, replacement: &str) {
        let removed = range.end - range.start;
        let added = replacement.len();
        self.text.replace_range(range.clone(), replacement);
        self.spans
            .retain(|span| span.range.end <= range.start || span.range.start >= range.end);
        for span in &mut self.spans {
            if span.range.start >= range.end {
                span.range.start = span.range.start + added - removed;
                span.range.end = span.range.end + added - removed;
            }
        }
        self.bump();
    }

    pub fn set_text(&mut self, text: impl Into<String>) {
        self.text = text.into();
        self.spans.clear();
        self.bump();
    }

    pub fn set_spans(&mut self, spans: Vec<TextSpan>) {
        self.spans = spans;
        self.bump();
    }

    /// Replaces every composition span, keeping the plain style spans.
    pub fn set_composition(&mut self, spans: Vec<TextSpan>) {
        self.spans.retain(|span| span.composition.is_none());
        self.spans.extend(spans);
        self.spans.sort_by_key(|span| span.range.start);
        self.bump();
    }

    /// True when any span is currently under IME composition.
    pub fn has_composition(&self) -> bool {
        self.spans.iter().any(|span| span.composition.is_some())
    }

    fn bump(&mut self) {
        self.revision = self.revision.next();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_mutating_method_bumps_the_revision() {
        let mut source = TextSource::new("hello");
        let start = source.revision();
        source.replace_range(0..1, "H");
        assert!(source.revision() > start, "replace_range must bump");
        let after_replace = source.revision();
        source.set_spans(Vec::new());
        assert!(source.revision() > after_replace, "set_spans must bump");
        let after_spans = source.revision();
        source.set_composition(Vec::new());
        assert!(source.revision() > after_spans, "set_composition must bump");
        let after_composition = source.revision();
        source.set_text("bye");
        assert!(source.revision() > after_composition, "set_text must bump");
        assert_eq!(source.text(), "bye");
    }

    #[test]
    fn replacing_a_range_shifts_the_spans_after_it() {
        let mut source = TextSource::new("abcdef");
        source.set_spans(vec![TextSpan {
            range: 4..6,
            style: TextStyle::default(),
            composition: None,
        }]);
        source.replace_range(0..2, "XYZW");
        assert_eq!(source.text(), "XYZWcdef");
        assert_eq!(source.spans()[0].range, 6..8);
    }
}
