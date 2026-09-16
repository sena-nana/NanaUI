//! Shaped output: runs of glyphs with their clusters, in logical order.
//!
//! Nothing here is derived. Advances come from the shaper and clusters come
//! from the shaper; neither is recomputed by summing glyphs or by re-running
//! grapheme segmentation, because a disagreement between the two is exactly
//! the kind of finding this IR exists to surface.

use crate::id::{FontId, ShapeRunId};
use crate::metrics::RunMetrics;
use serde::{Deserialize, Serialize};
use std::ops::Range;

/// Resolved direction of one run.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum RunDirection {
    #[default]
    Ltr,
    Rtl,
}

impl RunDirection {
    /// Even BiDi levels are left-to-right.
    pub const fn from_bidi_level(level: u8) -> Self {
        if level.is_multiple_of(2) {
            Self::Ltr
        } else {
            Self::Rtl
        }
    }

    pub const fn is_rtl(self) -> bool {
        matches!(self, Self::Rtl)
    }
}

/// An ISO 15924 script tag, e.g. `*b"Latn"`.
///
/// Four bytes rather than a string: it is an id, and run keys need it `Copy`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ScriptTag(pub [u8; 4]);

impl ScriptTag {
    pub const LATIN: Self = Self(*b"Latn");
    pub const ARABIC: Self = Self(*b"Arab");
    /// What an engine reports when it does not segment by script.
    pub const UNKNOWN: Self = Self(*b"Zzzz");

    pub fn as_str(&self) -> &str {
        std::str::from_utf8(&self.0).unwrap_or("Zzzz")
    }
}

impl Default for ScriptTag {
    fn default() -> Self {
        Self::UNKNOWN
    }
}

/// Per-glyph facts a renderer or a diff needs but cannot re-derive.
///
/// [`MISSING`](Self::MISSING) and [`FALLBACK_FONT`](Self::FALLBACK_FONT) are
/// load-bearing: they let the missing-glyph and fallback corpus rows assert
/// semantics rather than "the glyph id happened to be 0".
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub struct GlyphFlags(pub u8);

impl GlyphFlags {
    pub const NONE: Self = Self(0);
    /// Resolved to `.notdef`.
    pub const MISSING: Self = Self(1 << 0);
    /// Came from a face other than the requested family.
    pub const FALLBACK_FONT: Self = Self(1 << 1);

    pub const fn contains(self, flag: Self) -> bool {
        self.0 & flag.0 == flag.0
    }

    #[must_use]
    pub const fn with(self, flag: Self) -> Self {
        Self(self.0 | flag.0)
    }

    pub fn set(&mut self, flag: Self, on: bool) {
        if on {
            self.0 |= flag.0;
        } else {
            self.0 &= !flag.0;
        }
    }
}

/// One positioned glyph.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct ShapedGlyph {
    /// `u32` rather than `u16`. OpenType ids fit in 16 bits today, but this is
    /// a long-term cross-layer id and widening it later would be an ABI break.
    pub glyph_id: u32,
    /// Byte offsets of the cluster in the source text, as the shaper reported
    /// them. Never recomputed from grapheme boundaries.
    pub cluster: u32,
    pub cluster_end: u32,
    pub advance_px: f32,
    #[serde(default)]
    pub advance_y_px: f32,
    /// In physical px, not EM units.
    #[serde(default)]
    pub offset_x_px: f32,
    #[serde(default)]
    pub offset_y_px: f32,
    #[serde(default)]
    pub flags: GlyphFlags,
}

/// One shaped run: a maximal span of one font, direction and script.
///
/// `glyphs` are in **visual** (left-to-right) order, the HarfBuzz convention,
/// so `cluster` decreases across an RTL run and a glyph's x is always the run
/// origin plus the prefix sum of the advances before it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ShapedRun {
    #[serde(default)]
    pub id: ShapeRunId,
    /// Byte range in the source text. Logical order, not visual order.
    pub source: Range<usize>,
    pub direction: RunDirection,
    /// The Unicode BiDi embedding level verbatim.
    ///
    /// `direction` is derivable from it, but carrying both lets a diff say
    /// which of the two drifted.
    pub bidi_level: u8,
    #[serde(default)]
    pub script: ScriptTag,
    pub font: FontId,
    pub font_size_px: f32,
    pub glyphs: Vec<ShapedGlyph>,
    /// As reported by the engine, never re-derived from `glyphs`. A run whose
    /// advance disagrees with the sum of its glyphs is a finding.
    pub advance_px: f32,
    /// Left edge of the run in layout space.
    ///
    /// Assigned by **layout**, not by shaping: a shape cache keeps every other
    /// field across a relayout and layout rewrites this one. It is stored
    /// rather than derived because alignment and trailing whitespace mean runs
    /// are not always laid end to end.
    #[serde(default)]
    pub origin_x_px: f32,
    #[serde(default)]
    pub metrics: RunMetrics,
}

impl ShapedRun {
    /// Sum of the glyph advances. Compare against [`Self::advance_px`]; do not
    /// substitute for it.
    pub fn glyph_advance_sum_px(&self) -> f32 {
        self.glyphs.iter().map(|glyph| glyph.advance_px).sum()
    }

    /// Left edge of each glyph's advance cell, in layout space and visual
    /// order, paired with the glyph.
    pub fn glyph_cells(&self) -> impl Iterator<Item = (f32, &ShapedGlyph)> {
        let mut cursor = self.origin_x_px;
        self.glyphs.iter().map(move |glyph| {
            let left = cursor;
            cursor += glyph.advance_px;
            (left, glyph)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bidi_levels_map_to_direction_by_parity() {
        assert_eq!(RunDirection::from_bidi_level(0), RunDirection::Ltr);
        assert_eq!(RunDirection::from_bidi_level(1), RunDirection::Rtl);
        assert_eq!(RunDirection::from_bidi_level(2), RunDirection::Ltr);
    }

    #[test]
    fn glyph_flags_compose_and_read_back_independently() {
        let mut flags = GlyphFlags::NONE.with(GlyphFlags::MISSING);
        assert!(flags.contains(GlyphFlags::MISSING));
        assert!(!flags.contains(GlyphFlags::FALLBACK_FONT));
        flags.set(GlyphFlags::FALLBACK_FONT, true);
        assert!(flags.contains(GlyphFlags::MISSING));
        assert!(flags.contains(GlyphFlags::FALLBACK_FONT));
        flags.set(GlyphFlags::MISSING, false);
        assert!(!flags.contains(GlyphFlags::MISSING));
        assert!(flags.contains(GlyphFlags::FALLBACK_FONT));
    }
}
