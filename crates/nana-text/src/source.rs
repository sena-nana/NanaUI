//! The authored text plus its style spans and a revision that cannot be dodged.
//!
//! The fields are private on purpose: the only way to change the bytes is a
//! method that bumps [`TextRevision`], which is what makes the revision worth
//! caching on.

use crate::id::TextRevision;
use crate::style::TextStyle;
use serde::{Deserialize, Serialize};
use std::hash::{DefaultHasher, Hasher};
use std::ops::Range;
use std::sync::{Arc, OnceLock};

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
///
/// The text is an `Arc<str>`, so a shape cache can keep it as a key without
/// copying a byte, and its content hash is computed at most once per revision.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TextSource {
    text: Arc<str>,
    spans: Vec<TextSpan>,
    revision: TextRevision,
    /// Hash of `text`, filled on first use and reset by every mutation.
    #[serde(skip)]
    content_hash: OnceLock<u64>,
}

/// Equality is over the text, spans and revision; whether the hash happens to
/// be memoized yet is not part of a source's value.
impl PartialEq for TextSource {
    fn eq(&self, other: &Self) -> bool {
        self.text == other.text && self.spans == other.spans && self.revision == other.revision
    }
}

impl Default for TextSource {
    fn default() -> Self {
        Self::new(String::new())
    }
}

impl TextSource {
    pub fn new(text: impl Into<Arc<str>>) -> Self {
        Self {
            text: text.into(),
            spans: Vec::new(),
            revision: TextRevision::INITIAL,
            content_hash: OnceLock::new(),
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

    /// The shared text, for a cache that keys on it without copying.
    pub(crate) fn shared_text(&self) -> &Arc<str> {
        &self.text
    }

    /// Content hash of the text, and whether this call had to compute it.
    ///
    /// The same content hashes the same in every source and every process
    /// (`DefaultHasher::new` has fixed keys), so two widgets showing the same
    /// label land in the same cache bucket. A hash is only a bucket: equality
    /// is still decided on the bytes.
    pub(crate) fn content_hash(&self) -> (u64, bool) {
        let mut computed = false;
        let hash = *self.content_hash.get_or_init(|| {
            computed = true;
            let mut hasher = DefaultHasher::new();
            hasher.write(self.text.as_bytes());
            hasher.finish()
        });
        (hash, computed)
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
        let mut text = String::with_capacity(self.text.len() - removed + added);
        text.push_str(&self.text[..range.start]);
        text.push_str(replacement);
        text.push_str(&self.text[range.end..]);
        self.text = text.into();
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

    pub fn set_text(&mut self, text: impl Into<Arc<str>>) {
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
        self.content_hash = OnceLock::new();
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
    fn the_content_hash_is_computed_once_per_revision_and_shared_by_equal_text() {
        let mut source = TextSource::new("label");
        let (first, computed) = source.content_hash();
        assert!(computed);
        assert_eq!(source.content_hash(), (first, false));
        assert_eq!(TextSource::new("label").content_hash().0, first);

        source.set_spans(Vec::new());
        let (same_text, recomputed) = source.content_hash();
        assert!(recomputed, "a mutation resets the memo");
        assert_eq!(same_text, first);
        source.set_text("other");
        assert_ne!(source.content_hash().0, first);
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
