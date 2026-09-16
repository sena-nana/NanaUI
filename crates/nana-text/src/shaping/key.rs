//! What a shape result depends on, and nothing else.
//!
//! In: text content, span ranges and their shaping-relevant style (family,
//! size, weight, slant, letter spacing, features, variations, kerning),
//! direction, language, device scale, font generation.
//!
//! Out, deliberately: widget identity, [`TextRevision`](crate::TextRevision)
//! (two sources with equal text share an entry), line height, every
//! [`TextConstraints`](crate::TextConstraints) field except direction and
//! scale, and all paint (colour, opacity, transform, which `TextStyle` does not
//! even carry). A change to any of those must not reshape.
//!
//! The text is held as the source's own `Arc<str>`: building a key copies no
//! bytes, hashing uses the source's per-revision memo, and equality falls back
//! to comparing bytes, so a hash collision is never mistaken for a hit.

use crate::font::{FontFeatures, FontVariations, LanguageTag, canonical_f32_bits};
use crate::id::FontGeneration;
use crate::source::{TextSource, TextSpan};
use crate::style::TextStyle;
use nana_ui_core::FontKerningSpec;
use std::hash::{Hash, Hasher};
use std::ops::Range;
use std::sync::Arc;

/// The shaping-relevant subset of a [`TextStyle`], at physical size.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) struct StyleKey {
    family: Option<Arc<str>>,
    size_bits: u32,
    weight: u16,
    italic: bool,
    letter_spacing_bits: u32,
    features: FontFeatures,
    variations: FontVariations,
    kerning_off: bool,
}

impl StyleKey {
    pub fn new(style: &TextStyle, scale: f32) -> Self {
        Self {
            family: style.font_family.clone(),
            size_bits: canonical_f32_bits(style.font_size_px * scale),
            weight: style.font_weight,
            italic: style.italic,
            letter_spacing_bits: canonical_f32_bits(style.letter_spacing_px * scale),
            features: FontFeatures::from_settings(&style.features),
            variations: FontVariations::from_settings(&style.variations),
            kerning_off: style.kerning == FontKerningSpec::None,
        }
    }
}

#[derive(Debug, Clone)]
pub(crate) struct ShapeKey {
    text: Arc<str>,
    text_hash: u64,
    base: StyleKey,
    /// Authored spans as given, with whether each is a composition span:
    /// composition spans win where spans overlap, so the flag decides which
    /// style shapes those bytes. Which composition segment it is does not.
    spans: Vec<(Range<usize>, bool, StyleKey)>,
    rtl: bool,
    language: Option<LanguageTag>,
    scale_bits: u32,
    /// The font system and its generation: `FontId`s in a result are only
    /// meaningful for the system that issued them.
    epoch: FontEpoch,
}

/// A font system's identity and generation.
pub(crate) type FontEpoch = (u64, FontGeneration);

impl ShapeKey {
    #[expect(
        clippy::too_many_arguments,
        reason = "every shaping input is a key field"
    )]
    pub fn new(
        source: &TextSource,
        text_hash: u64,
        base: &TextStyle,
        spans: &[TextSpan],
        rtl: bool,
        language: Option<&LanguageTag>,
        scale: f32,
        epoch: FontEpoch,
    ) -> Self {
        Self {
            text: Arc::clone(source.shared_text()),
            text_hash,
            base: StyleKey::new(base, scale),
            spans: spans
                .iter()
                .map(|span| {
                    (
                        span.range.clone(),
                        span.composition.is_some(),
                        StyleKey::new(&span.style, scale),
                    )
                })
                .collect(),
            rtl,
            language: language.cloned(),
            scale_bits: canonical_f32_bits(scale),
            epoch,
        }
    }

    pub fn epoch(&self) -> FontEpoch {
        self.epoch
    }

    /// Bytes this key keeps alive, charged to the cache budget.
    pub fn retained_bytes(&self) -> usize {
        self.text.len()
            + self.spans.capacity() * std::mem::size_of::<(Range<usize>, bool, StyleKey)>()
    }
}

impl Hash for ShapeKey {
    fn hash<H: Hasher>(&self, state: &mut H) {
        // The memoized content hash stands in for the bytes.
        self.text_hash.hash(state);
        self.base.hash(state);
        self.spans.hash(state);
        self.rtl.hash(state);
        self.language.hash(state);
        self.scale_bits.hash(state);
        self.epoch.hash(state);
    }
}

impl PartialEq for ShapeKey {
    fn eq(&self, other: &Self) -> bool {
        self.text_hash == other.text_hash
            && self.rtl == other.rtl
            && self.scale_bits == other.scale_bits
            && self.epoch == other.epoch
            && self.base == other.base
            && self.spans == other.spans
            && self.language == other.language
            && (Arc::ptr_eq(&self.text, &other.text) || self.text == other.text)
    }
}

impl Eq for ShapeKey {}
