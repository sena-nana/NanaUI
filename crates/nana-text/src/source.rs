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

/// Line separators a space can stand in for byte-for-byte: each is one byte,
/// and so is the space that replaces it.
///
/// Every one-byte member of the two lists that end a line — UBA's
/// `PARAGRAPH_SEPARATORS` and UAX #14's `FORCED_BREAKS` — appears here, and
/// nothing else does; `every_one_byte_separator_can_fold` holds the three lists
/// to that. The rest are longer: `U+0085 NEL` is two bytes, `U+2028 LINE
/// SEPARATOR` and `U+2029 PARAGRAPH SEPARATOR` three. Folding one would move
/// every offset after it, so they stay line breaks whatever `preserve_lines`
/// says.
const FOLDED_SEPARATORS: [char; 7] = ['\n', '\r', '\u{1c}', '\u{1d}', '\u{1e}', '\u{b}', '\u{c}'];

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

/// The character an inline object stands in the text as: U+FFFC OBJECT
/// REPLACEMENT CHARACTER. One cluster, so a caret steps over an object in
/// one move and a selection takes it whole.
pub const OBJECT_REPLACEMENT: char = '\u{FFFC}';

/// How much room an inline object takes on its line, in logical px.
///
/// The object sits on the baseline: `ascent_px` above it, `descent_px` below.
/// A zero width is an object that takes no room at all (an editor's marker),
/// so text around it breaks exactly as it would without it.
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
pub struct InlineObjectMetrics {
    pub width_px: f32,
    pub ascent_px: f32,
    pub descent_px: f32,
    /// What the object gives a line deciding where to break (Issue #211):
    /// a gap it keeps after itself that may close up, and whether a line may
    /// break before or after it. `None`: no gap, and breaks as UAX #14 says.
    #[serde(default)]
    pub envelope: Option<InlineEnvelope>,
}

/// An inline object's part in the CJK line decision. The decision never
/// looks inside the object: it sees a gap, how much of it may close, and
/// where it may break.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct InlineEnvelope {
    /// The gap the object keeps after itself, in logical px.
    pub gap_px: f32,
    /// How much of that gap a short line may close up; dropped at a line end.
    pub gap_shrink_px: f32,
    pub break_before: bool,
    pub break_after: bool,
}

impl Default for InlineEnvelope {
    fn default() -> Self {
        Self {
            gap_px: 0.0,
            gap_shrink_px: 0.0,
            break_before: true,
            break_after: true,
        }
    }
}

/// An object placed in the text: a sticker, an image, an editor chip.
///
/// The text holds one [`OBJECT_REPLACEMENT`] at `offset`; this entry says how
/// large it is. What it *is* — an image, a texture — is the caller's: the
/// engine only needs its box. Its size is read at layout, never at shaping, so
/// resizing an object lays the paragraph out again without shaping it again.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct InlineObject {
    /// Byte offset of the object's U+FFFC in [`TextSource::text`].
    pub offset: usize,
    /// The caller's name for the object, handed back in
    /// [`crate::PlacedObject::id`].
    pub id: u64,
    pub metrics: InlineObjectMetrics,
    #[serde(default)]
    pub flow: nana_ui_core::RichObjectFlow,
    #[serde(default)]
    pub fit_width: bool,
}

/// Whether an object shares its line with the text around it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ObjectFlow {
    /// Among the text, broken around like any cluster.
    #[default]
    Inline,
    /// On a line of its own. The text before it ends a line there and the
    /// text after it starts a new one, without a character standing for
    /// either break: the U+FFFC stays the object's only byte, and a caret at
    /// either edge has a place on both lines, as at a soft wrap.
    Block,
}

/// How an object of [`TextSource::objects`] sits among the lines, when that
/// is not the default: inline, at its own size.
///
/// Kept beside the objects rather than in [`InlineObject`], as
/// [`ObjectLabel`] is: a source with none pays nothing, and an object built
/// without one is inline at its own size.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ObjectPlacement {
    /// Byte offset of the object's U+FFFC in [`TextSource::text`].
    pub offset: usize,
    #[serde(default)]
    pub flow: ObjectFlow,
    /// Scaled down, keeping its aspect, when wider than the room a line has
    /// for it: the line budget, less the gap its envelope keeps. Never scaled
    /// up. Its `min-content` is then nothing — it shrinks with its box, as
    /// an image with CSS `max-width: 100%` does — while `max-content` stays
    /// its own width. A labelled object is sized by its label and does not
    /// scale.
    #[serde(default)]
    pub fit_width: bool,
}

impl ObjectPlacement {
    /// The object at `offset`, flowing as `flow` at its own size.
    pub fn new(offset: usize, flow: ObjectFlow) -> Self {
        Self {
            offset,
            flow,
            fit_width: false,
        }
    }

    /// Whether it scales down to fit its line's room.
    #[must_use]
    pub fn fit_width(mut self, fit: bool) -> Self {
        self.fit_width = fit;
        self
    }

    /// Inline at its own size: what an object without a placement does.
    fn is_default(&self) -> bool {
        self.flow == ObjectFlow::Inline && !self.fit_width
    }
}

/// A ruby annotation: small text set above a base range (furigana, pinyin).
///
/// The base is laid out as one unit — no line breaks inside it — and is
/// spaced out when the annotation is wider than it. The annotation is shaped
/// once, at [`RUBY_SCALE`] of the base's size, and placed centred above it.
/// Horizontal text only: a vertical layout drops annotations and says so.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RubySpan {
    /// Byte range of the base in [`TextSource::text`].
    pub range: Range<usize>,
    pub text: Arc<str>,
}

/// An annotation's size relative to its base.
pub const RUBY_SCALE: f32 = 0.5;

/// Emphasis marks (CSS `text-emphasis`) over a byte range: one mark beside
/// each character of it that takes one.
///
/// The mark is the first grapheme cluster of [`Self::mark`], shaped as
/// ordinary text at [`EMPHASIS_SCALE`] of the size of the character it
/// stands on and centred on that character's advance. Separators,
/// punctuation, control and format characters and inline objects take no
/// mark. A line holding marks grows by their height on their side, as it
/// does for ruby; the characters themselves are neither reshaped nor moved.
/// Horizontal text only: a vertical layout drops marks and says so.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EmphasisSpan {
    /// Byte range in [`TextSource::text`].
    pub range: Range<usize>,
    /// The mark: for a CSS keyword, the character
    /// [`nana_ui_core::TextEmphasisStyle::mark_text`] names.
    pub mark: Arc<str>,
    pub position: nana_ui_core::TextEmphasisPosition,
}

/// An emphasis mark's size relative to the character it stands on.
pub const EMPHASIS_SCALE: f32 = 0.5;

/// A label an inline object shows inside itself: an editor's marker tag
/// ("pause", "sound: coin"), or a name with a picture before it. The object
/// takes the label's room — shaped at [`Self::scale`] of the text around it,
/// plus padding and the icon's square — instead of its own
/// [`InlineObjectMetrics`], and the layout places the label's runs inside it
/// ([`crate::PlacedLabel`]).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ObjectLabel {
    /// Byte offset of the object's U+FFFC in [`TextSource::text`].
    pub offset: usize,
    pub text: Arc<str>,
    /// Size relative to the text around the object.
    #[serde(default = "label_scale")]
    pub scale: f32,
    /// Weight; `None` takes the text's around it, held to 500..=700.
    #[serde(default)]
    pub weight: Option<u16>,
    /// Ink, straight sRGB with alpha; `None` is the light tag ink.
    #[serde(default)]
    pub color: Option<[f32; 4]>,
    /// Keep a square before the text for a picture
    /// ([`crate::PlacedLabel::icon`]).
    #[serde(default)]
    pub icon: bool,
}

fn label_scale() -> f32 {
    LABEL_SCALE
}

impl ObjectLabel {
    /// A plain tag label: small, medium weight, light ink, no icon.
    pub fn new(offset: usize, text: impl Into<Arc<str>>) -> Self {
        Self {
            offset,
            text: text.into(),
            scale: LABEL_SCALE,
            weight: None,
            color: None,
            icon: false,
        }
    }

    /// What the layout reads besides the shaped text: the icon square and
    /// the ink, as bits for a cache key.
    pub fn layout_bits(&self) -> [u32; 5] {
        let [r, g, b, a] = self.color.unwrap_or([-1.0; 4]);
        [
            u32::from(self.icon),
            r.to_bits(),
            g.to_bits(),
            b.to_bits(),
            a.to_bits(),
        ]
    }
}

/// A label's size relative to the text around its object.
pub const LABEL_SCALE: f32 = 0.62;

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

/// Span ranges snapped to grapheme cluster starts, and the rule for which span
/// governs a byte.
///
/// Shaping and layout must answer this identically. A span boundary that falls
/// inside a grapheme cluster is snapped back to the cluster's start — the
/// shaper cannot split a cluster — so a layout reading the *raw* range would
/// size a line box from the base style for a run the shaper shaped at the
/// span's size, and the glyphs would overflow the line box they were measured
/// into.
pub(crate) struct SnappedSpans<'a> {
    spans: Vec<(Range<usize>, &'a TextSpan)>,
    text_len: usize,
}

impl<'a> SnappedSpans<'a> {
    /// `cluster_starts` are the byte offsets grapheme clusters start at.
    pub fn new(spans: &'a [TextSpan], text_len: usize, cluster_starts: &[usize]) -> Self {
        let snap = |byte: usize| -> usize {
            let byte = byte.min(text_len);
            if byte == text_len {
                return byte;
            }
            let index = cluster_starts.partition_point(|start| *start <= byte);
            cluster_starts[index.saturating_sub(1)]
        };
        Self {
            spans: spans
                .iter()
                .map(|span| (snap(span.range.start)..snap(span.range.end), span))
                .collect(),
            text_len,
        }
    }

    /// Every offset where the governing style can change, `0` and the text's
    /// end included.
    pub fn cuts(&self) -> Vec<usize> {
        let mut cuts: Vec<usize> = Vec::with_capacity(self.spans.len() * 2 + 2);
        cuts.push(0);
        cuts.push(self.text_len);
        for (range, _) in &self.spans {
            cuts.push(range.start);
            cuts.push(range.end);
        }
        cuts.sort_unstable();
        cuts.dedup();
        cuts
    }

    /// The style governing `byte`: the last composition span covering it, then
    /// the last plain span, then the base.
    pub fn style_at(&self, byte: usize, base: &'a TextStyle) -> &'a TextStyle {
        let covering =
            |(range, _): &&(Range<usize>, &'a TextSpan)| range.start <= byte && byte < range.end;
        self.spans
            .iter()
            .rfind(|entry| covering(entry) && entry.1.composition.is_some())
            .or_else(|| self.spans.iter().rfind(covering))
            .map_or(base, |(_, span)| &span.style)
    }
}

/// Authored text, its spans, and the revision they are at.
///
/// The text is an `Arc<str>`, so a shape cache can keep it as a key without
/// copying a byte, and its content hash is computed at most once per revision.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TextSource {
    text: Arc<str>,
    #[serde(default)]
    paragraph_alignments: Vec<(usize, Option<nana_ui_core::TextAlignSpec>)>,
    spans: Vec<TextSpan>,
    /// Sorted by offset; each names a U+FFFC of `text`.
    #[serde(default)]
    objects: Vec<InlineObject>,
    /// Sorted, non-overlapping.
    #[serde(default)]
    rubies: Vec<RubySpan>,
    /// Sorted by offset; each names an object of `objects`.
    #[serde(default)]
    labels: Vec<ObjectLabel>,
    /// Sorted by offset; each names an object of `objects`, and none is the
    /// default placement.
    #[serde(default)]
    placements: Vec<ObjectPlacement>,
    /// Sorted, non-overlapping.
    #[serde(default)]
    emphasis: Vec<EmphasisSpan>,
    revision: TextRevision,
    /// Hash of `text`, filled on first use and reset by every mutation.
    #[serde(skip)]
    content_hash: OnceLock<u64>,
    /// This source read under `preserve_lines: false`, folded at most once per
    /// revision. `None` inside the cell means there was nothing to fold.
    ///
    /// Memoized here rather than in the engine because that is where the
    /// lifetime already is: a folded copy is a new string whose content hash
    /// would otherwise be recomputed for every frame that lays this source out.
    #[serde(skip)]
    folded: OnceLock<Option<Box<TextSource>>>,
}

/// Equality is over the text, spans and revision; whether the hash happens to
/// be memoized yet is not part of a source's value.
impl PartialEq for TextSource {
    fn eq(&self, other: &Self) -> bool {
        self.text == other.text
            && self.paragraph_alignments == other.paragraph_alignments
            && self.spans == other.spans
            && self.objects == other.objects
            && self.rubies == other.rubies
            && self.labels == other.labels
            && self.placements == other.placements
            && self.emphasis == other.emphasis
            && self.revision == other.revision
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
            paragraph_alignments: Vec::new(),
            spans: Vec::new(),
            objects: Vec::new(),
            rubies: Vec::new(),
            labels: Vec::new(),
            placements: Vec::new(),
            emphasis: Vec::new(),
            revision: TextRevision::INITIAL,
            content_hash: OnceLock::new(),
            folded: OnceLock::new(),
        }
    }

    /// Paragraph-start offsets and their optional authored alignment.
    pub fn paragraph_alignments(&self) -> &[(usize, Option<nana_ui_core::TextAlignSpec>)] {
        &self.paragraph_alignments
    }

    pub fn set_paragraph_alignments(
        &mut self,
        mut values: Vec<(usize, Option<nana_ui_core::TextAlignSpec>)>,
    ) {
        values.retain(|(at, _)| {
            *at <= self.text.len() && (*at == 0 || self.text.as_bytes()[at - 1] == b'\n')
        });
        values.sort_by_key(|(at, _)| *at);
        values.dedup_by_key(|(at, _)| *at);
        let mut offset = 0;
        self.paragraph_alignments = self
            .text
            .split('\n')
            .map(|text| {
                let start = offset;
                offset += text.len() + 1;
                let align = values
                    .binary_search_by_key(&start, |(at, _)| *at)
                    .ok()
                    .and_then(|i| values[i].1);
                (start, align)
            })
            .collect();
        self.bump();
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

    /// The inline objects, sorted by offset.
    pub fn objects(&self) -> &[InlineObject] {
        &self.objects
    }

    /// The ruby annotations, sorted by base.
    pub fn rubies(&self) -> &[RubySpan] {
        &self.rubies
    }

    /// The emphasis ranges, sorted.
    pub fn emphasis(&self) -> &[EmphasisSpan] {
        &self.emphasis
    }

    /// The object labels, sorted by offset.
    pub fn labels(&self) -> &[ObjectLabel] {
        &self.labels
    }

    /// Index into [`Self::labels`] of the label of the object at byte
    /// `offset`, if it has one.
    pub fn label_index(&self, offset: usize) -> Option<usize> {
        self.labels
            .binary_search_by_key(&offset, |label| label.offset)
            .ok()
    }

    /// Replaces the object labels. One not on an object of
    /// [`Self::objects`], or empty, is dropped. Set the objects first.
    pub fn set_labels(&mut self, mut labels: Vec<ObjectLabel>) {
        labels.retain(|label| !label.text.is_empty() && self.object_at(label.offset).is_some());
        labels.sort_by_key(|label| label.offset);
        labels.dedup_by_key(|label| label.offset);
        self.labels = labels;
        self.bump();
    }

    /// The object placements, sorted by offset. An object with none is
    /// inline at its own size.
    pub fn placements(&self) -> &[ObjectPlacement] {
        &self.placements
    }

    /// The placement of the object at byte `offset`, if it has one.
    pub fn placement_at(&self, offset: usize) -> Option<&ObjectPlacement> {
        self.placements
            .binary_search_by_key(&offset, |placement| placement.offset)
            .ok()
            .map(|index| &self.placements[index])
    }

    /// Replaces the object placements. One not on an object of
    /// [`Self::objects`], or the default placement, is dropped. Set the
    /// objects first.
    pub fn set_placements(&mut self, mut placements: Vec<ObjectPlacement>) {
        placements.retain(|placement| {
            !placement.is_default() && self.object_at(placement.offset).is_some()
        });
        placements.sort_by_key(|placement| placement.offset);
        placements.dedup_by_key(|placement| placement.offset);
        self.placements = placements;
        self.bump();
    }

    /// Replaces the ruby annotations. One whose base is empty, past the text
    /// or off character boundaries is dropped, and so is one overlapping an
    /// earlier one.
    pub fn set_rubies(&mut self, mut rubies: Vec<RubySpan>) {
        rubies.retain(|ruby| {
            ruby.range.start < ruby.range.end
                && ruby.range.end <= self.text.len()
                && self.text.is_char_boundary(ruby.range.start)
                && self.text.is_char_boundary(ruby.range.end)
                && !ruby.text.is_empty()
        });
        rubies.sort_by_key(|ruby| ruby.range.start);
        let mut kept: Vec<RubySpan> = Vec::with_capacity(rubies.len());
        for ruby in rubies {
            if kept
                .last()
                .is_none_or(|last| last.range.end <= ruby.range.start)
            {
                kept.push(ruby);
            }
        }
        self.rubies = kept;
        self.bump();
    }

    /// Replaces the emphasis ranges. One that is empty, past the text, off
    /// character boundaries or without a mark is dropped, and so is one
    /// overlapping an earlier one.
    pub fn set_emphasis(&mut self, mut emphasis: Vec<EmphasisSpan>) {
        emphasis.retain(|span| {
            span.range.start < span.range.end
                && span.range.end <= self.text.len()
                && self.text.is_char_boundary(span.range.start)
                && self.text.is_char_boundary(span.range.end)
                && !span.mark.is_empty()
        });
        emphasis.sort_by_key(|span| span.range.start);
        let mut kept: Vec<EmphasisSpan> = Vec::with_capacity(emphasis.len());
        for span in emphasis {
            if kept
                .last()
                .is_none_or(|last| last.range.end <= span.range.start)
            {
                kept.push(span);
            }
        }
        self.emphasis = kept;
        self.bump();
    }

    /// The object whose U+FFFC sits at byte `offset`, if any.
    pub fn object_at(&self, offset: usize) -> Option<&InlineObject> {
        self.objects
            .binary_search_by_key(&offset, |object| object.offset)
            .ok()
            .map(|index| &self.objects[index])
    }

    /// Replaces the inline objects. An entry whose offset does not hold a
    /// U+FFFC is dropped: it would size a character that is not an object.
    pub fn set_objects(&mut self, mut objects: Vec<InlineObject>) {
        objects.retain(|object| {
            self.text
                .get(object.offset..)
                .is_some_and(|rest| rest.starts_with(OBJECT_REPLACEMENT))
        });
        objects.sort_by_key(|object| object.offset);
        objects.dedup_by_key(|object| object.offset);
        self.objects = objects;
        self.bump();
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
        let alignments = (!self.paragraph_alignments.is_empty()).then(|| {
            let first = self.text[..range.start]
                .bytes()
                .filter(|b| *b == b'\n')
                .count();
            let last = self.text[..range.end]
                .bytes()
                .filter(|b| *b == b'\n')
                .count();
            let mut starts = vec![0];
            starts.extend(self.text.match_indices('\n').map(|(at, _)| at + 1));
            let mut values = starts
                .iter()
                .map(|at| {
                    self.paragraph_alignments
                        .binary_search_by_key(at, |(offset, _)| *offset)
                        .ok()
                        .and_then(|i| self.paragraph_alignments[i].1)
                })
                .collect::<Vec<_>>();
            let align = values[first];
            values.splice(
                first + 1..last + 1,
                std::iter::repeat_n(align, replacement.bytes().filter(|b| *b == b'\n').count()),
            );
            values
        });
        let removed = range.end - range.start;
        let added = replacement.len();
        let mut text = String::with_capacity(self.text.len() - removed + added);
        text.push_str(&self.text[..range.start]);
        text.push_str(replacement);
        text.push_str(&self.text[range.end..]);
        self.text = text.into();
        if let Some(values) = alignments {
            let mut offset = 0;
            self.paragraph_alignments = self
                .text
                .split('\n')
                .zip(values)
                .map(|(text, align)| {
                    let entry = (offset, align);
                    offset += text.len() + 1;
                    entry
                })
                .collect();
        }
        self.spans
            .retain(|span| span.range.end <= range.start || span.range.start >= range.end);
        for span in &mut self.spans {
            if span.range.start >= range.end {
                span.range.start = span.range.start + added - removed;
                span.range.end = span.range.end + added - removed;
            }
        }
        self.rubies
            .retain(|ruby| ruby.range.end <= range.start || ruby.range.start >= range.end);
        for ruby in &mut self.rubies {
            if ruby.range.start >= range.end {
                ruby.range.start = ruby.range.start + added - removed;
                ruby.range.end = ruby.range.end + added - removed;
            }
        }
        self.objects
            .retain(|object| object.offset < range.start || object.offset >= range.end);
        for object in &mut self.objects {
            if object.offset >= range.end {
                object.offset = object.offset + added - removed;
            }
        }
        // Emphasis is a style of the characters, not a unit: one holding the
        // edit takes the replacement in, one the edit cuts into loses the
        // part cut.
        for span in &mut self.emphasis {
            if span.range.end <= range.start {
                continue;
            }
            if span.range.start >= range.end {
                span.range.start = span.range.start + added - removed;
                span.range.end = span.range.end + added - removed;
            } else if span.range.start <= range.start && range.end <= span.range.end {
                span.range.end = span.range.end + added - removed;
            } else if span.range.start < range.start {
                span.range.end = range.start;
            } else {
                span.range.start = range.start + added;
                span.range.end = (span.range.end + added).saturating_sub(removed);
            }
        }
        self.emphasis
            .retain(|span| span.range.start < span.range.end);
        self.labels
            .retain(|label| label.offset < range.start || label.offset >= range.end);
        for label in &mut self.labels {
            if label.offset >= range.end {
                label.offset = label.offset + added - removed;
            }
        }
        self.placements
            .retain(|placement| placement.offset < range.start || placement.offset >= range.end);
        for placement in &mut self.placements {
            if placement.offset >= range.end {
                placement.offset = placement.offset + added - removed;
            }
        }
        self.bump();
    }

    pub fn set_text(&mut self, text: impl Into<Arc<str>>) {
        self.text = text.into();
        self.paragraph_alignments.clear();
        self.spans.clear();
        self.objects.clear();
        self.rubies.clear();
        self.labels.clear();
        self.placements.clear();
        self.emphasis.clear();
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

    /// This source with every one-byte line separator — `\n`, `\r`, VT and FF
    /// — replaced by a space, or `None` when it has none.
    ///
    /// `white-space: normal` (`TextConstraints::preserve_lines == false`) says
    /// an authored newline is a space rather than a line break. Shaping and
    /// line breaking must see the same bytes, so the fold happens before
    /// shaping — and each of these characters is one byte, as is the space, so
    /// every span range, cluster and caret offset still addresses the same
    /// character.
    ///
    /// The revision is kept: this is the same edit of the same text, read under
    /// different constraints, and a reader that treated it as a newer revision
    /// would invalidate caches that are not stale.
    ///
    /// `U+0085 NEL` (two bytes), `U+2028 LINE SEPARATOR` and `U+2029 PARAGRAPH
    /// SEPARATOR` (three each) are longer than a space, so folding one would
    /// move every offset after it. They stay line breaks whatever
    /// `preserve_lines` says, and layout treats them as such.
    ///
    /// A `\r\n` becomes **two** spaces, for the same reason. This engine does
    /// no whitespace collapsing at all — two authored spaces are two spaces —
    /// so a caller that wants CSS's collapsing normalizes its own text, where
    /// moving the offsets is its own business.
    ///
    /// The fold is computed at most once per revision and kept with the source,
    /// so laying the same text out every frame neither copies nor re-hashes it.
    pub fn with_folded_newlines(&self) -> Option<&Self> {
        self.folded
            .get_or_init(|| {
                self.text.contains(FOLDED_SEPARATORS).then(|| {
                    Box::new(Self {
                        paragraph_alignments: Vec::new(),
                        text: self.text.replace(FOLDED_SEPARATORS, " ").into(),
                        spans: self.spans.clone(),
                        objects: self.objects.clone(),
                        rubies: self.rubies.clone(),
                        labels: self.labels.clone(),
                        placements: self.placements.clone(),
                        emphasis: self.emphasis.clone(),
                        revision: self.revision,
                        content_hash: OnceLock::new(),
                        folded: OnceLock::new(),
                    })
                })
            })
            .as_deref()
    }

    /// True when any span is currently under IME composition.
    pub fn has_composition(&self) -> bool {
        self.spans.iter().any(|span| span.composition.is_some())
    }

    fn bump(&mut self) {
        self.revision = self.revision.next();
        self.content_hash = OnceLock::new();
        self.folded = OnceLock::new();
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

    /// The three lists cannot drift: everything that ends a line and fits in
    /// one byte folds, and nothing else does.
    #[test]
    fn every_one_byte_separator_can_fold() {
        use crate::layout::FORCED_BREAKS;
        use crate::shaping::PARAGRAPH_SEPARATORS;
        for separator in PARAGRAPH_SEPARATORS.iter().chain(FORCED_BREAKS.iter()) {
            assert_eq!(
                separator.len_utf8() == 1,
                FOLDED_SEPARATORS.contains(separator),
                "U+{:04X} is one byte: {}, folds: {}",
                *separator as u32,
                separator.len_utf8() == 1,
                FOLDED_SEPARATORS.contains(separator)
            );
        }
        for folded in FOLDED_SEPARATORS {
            assert_eq!(folded.len_utf8(), 1, "a fold must not move any offset");
            assert!(
                PARAGRAPH_SEPARATORS.contains(&folded) || FORCED_BREAKS.contains(&folded),
                "U+{:04X} folds although nothing ends a line at it",
                folded as u32
            );
        }
    }

    #[test]
    fn the_folded_reading_is_built_once_per_revision() {
        let mut source = TextSource::new("one\ntwo");
        let first = source
            .with_folded_newlines()
            .expect("the text has a newline")
            .text()
            .as_ptr();
        assert_eq!(
            source
                .with_folded_newlines()
                .expect("memoized")
                .text()
                .as_ptr(),
            first,
            "the same folded source comes back, not a fresh copy"
        );
        source.set_text("three\nfour");
        // The old fold is freed by the edit, so its address may be reused;
        // the memo is proven dropped by what the new fold reads.
        let refolded = source.with_folded_newlines().expect("still has a newline");
        assert_eq!(refolded.text(), "three four", "an edit drops the memo");
        assert_eq!(refolded.revision(), source.revision());
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
    fn folding_newlines_keeps_every_byte_offset_and_the_revision() {
        let mut source = TextSource::new("one\ntwo");
        source.set_spans(vec![TextSpan {
            range: 4..7,
            style: TextStyle::default(),
            composition: None,
        }]);
        let folded = source
            .with_folded_newlines()
            .expect("the text has a newline");
        assert_eq!(folded.text(), "one two");
        assert_eq!(folded.text().len(), source.text().len());
        assert_eq!(folded.spans()[0].range, 4..7);
        assert_eq!(
            folded.revision(),
            source.revision(),
            "folding is a reading of the same edit, not a new one"
        );
        assert!(
            TextSource::new("no breaks")
                .with_folded_newlines()
                .is_none()
        );
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
