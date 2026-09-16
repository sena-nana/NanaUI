//! The shaping authority: text and spans in, immutable [`ShapedRun`]s out
//! (Issue #91).
//!
//! ```text
//! TextSource + TextStyle + FontSystem
//!   → span normalization (snapped to grapheme boundaries)
//!   → grapheme / script / BiDi segmentation
//!   → font candidate selection (FontSystem::resolve_text, #90)
//!   → HarfRust shaping, with the item's surrounding text as context
//!   → .notdef fallback retry per cluster range
//!   → Arc<ShapedText>, cached under a ShapeKey
//! ```
//!
//! Line breaking, alignment and caret geometry are Phase 3: a shape result is
//! independent of width, so a relayout at a new width reuses it untouched.
//! `unicode-bidi` and `harfrust` are named only from the private `bidi` and
//! `opentype` modules.

mod bidi;
mod cache;
mod key;
mod opentype;
mod shaper;

/// Rule L2 over a sequence of embedding levels. Layout reorders a line's runs
/// with it after applying rule L1 to the line's trailing whitespace.
pub(crate) use bidi::visual_order as bidi_visual_order;
pub use cache::ShapeCacheBudget;
pub use shaper::{MAX_FALLBACK_CANDIDATES_PER_RANGE, MAX_FALLBACK_RETRIES_PER_ITEM, Shaper};

use crate::constraints::{TextConstraints, TextScale};
use crate::font::LanguageTag;
use crate::id::FontGeneration;
use crate::shape::ShapedRun;
use crate::source::TextSource;
use crate::style::TextStyle;
use nana_ui_core::DirSpec;
use serde::{Deserialize, Serialize};
use std::ops::Range;

/// One shaping call's inputs.
///
/// Built from [`TextConstraints`] with [`Self::new`], which takes only the two
/// fields shaping depends on (direction and device scale) — width, wrap,
/// max lines and the rest cannot reach the cache key.
#[derive(Debug, Clone, Copy)]
pub struct ShapeRequest<'a> {
    pub source: &'a TextSource,
    /// Applies wherever [`TextSource::spans`] leaves a gap.
    pub style: &'a TextStyle,
    /// Paragraph base direction, as CSS `direction` sets it.
    pub direction: DirSpec,
    pub language: Option<&'a LanguageTag>,
    pub scale: TextScale,
}

impl<'a> ShapeRequest<'a> {
    pub fn new(
        source: &'a TextSource,
        style: &'a TextStyle,
        constraints: &TextConstraints,
    ) -> Self {
        Self {
            source,
            style,
            direction: constraints.base_direction,
            language: None,
            scale: constraints.scale,
        }
    }

    #[must_use]
    pub fn with_language(mut self, language: Option<&'a LanguageTag>) -> Self {
        self.language = language;
        self
    }
}

/// One BiDi paragraph of the shaped text.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ShapedParagraph {
    /// Byte range, including the trailing paragraph separator if any.
    pub range: Range<usize>,
    /// Resolved paragraph embedding level (0 LTR, 1 RTL).
    pub base_level: u8,
}

/// The immutable result of shaping one source.
///
/// Shared behind an `Arc`: layout reads it and copies runs out when it needs
/// to assign origins, and never mutates the cached value.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ShapedText {
    /// In **logical** order. Each run carries its own `bidi_level`, which is
    /// what Phase 3 reorders lines with. Paragraph separators produce no run,
    /// so runs need not tile the text.
    pub runs: Vec<ShapedRun>,
    pub paragraphs: Vec<ShapedParagraph>,
    pub font_generation: FontGeneration,
}

impl ShapedText {
    pub fn glyph_count(&self) -> usize {
        self.runs.iter().map(|run| run.glyphs.len()).sum()
    }

    /// Visual order (rule L2) of `runs[range]`, as indices into `runs`.
    ///
    /// For one line of runs whose trailing whitespace levels have already been
    /// reset (rule L1), which is Phase 3's job.
    pub fn visual_order(&self, range: Range<usize>) -> Vec<usize> {
        let levels: Vec<u8> = self.runs[range.clone()]
            .iter()
            .map(|run| run.bidi_level)
            .collect();
        bidi::visual_order(&levels)
            .into_iter()
            .map(|index| range.start + index)
            .collect()
    }
}

/// Work counts for shaping. Accumulate until [`Shaper::reset_counters`];
/// `shape_cache_bytes` is a gauge read when [`Shaper::counters`] is called.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct ShapeCounters {
    pub shape_requests: usize,
    pub shape_runs_created: usize,
    pub shape_glyphs_created: usize,
    pub shape_cache_hits: usize,
    pub shape_cache_misses: usize,
    pub shape_cache_evictions: usize,
    pub shape_cache_bytes: usize,
    pub shape_cache_entries: usize,
    /// Level runs (maximal equal-level cluster sequences) segmented on misses.
    pub bidi_runs: usize,
    /// Script runs segmented on misses.
    pub script_runs: usize,
    /// Cluster ranges re-shaped with another face after shaping to `.notdef`.
    pub fallback_retries: usize,
    /// Candidate faces considered for those retries.
    pub fallback_fonts_examined: usize,
    /// Text bytes fed to the content hash. Once per source revision.
    pub text_bytes_hashed: usize,
    /// Text bytes copied to build or store a key. The key shares the
    /// source's `Arc<str>`, so this stays 0 unless that changes.
    pub text_bytes_cloned_for_shape: usize,
    /// Text bytes that produced no run because the font system has no face at
    /// all. Paragraph separators are not counted.
    pub text_bytes_unshaped: usize,
}
