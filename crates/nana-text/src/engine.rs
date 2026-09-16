//! The seam a real text engine implements.
//!
//! `nana-text` ships **no** implementation of this trait. That is the
//! mechanical meaning of "Phase 0 does not switch the product text path": a
//! product crate that wanted to use it would have nothing to construct. The
//! cosmic reference implementation lives under `tests/`, where no product
//! dependency edge can reach it.

use crate::constraints::TextConstraints;
use crate::counters::TextWorkCounters;
use crate::id::FontGeneration;
use crate::layout::TextLayout;
use crate::source::TextSource;
use crate::style::{TextKind, TextStyle};

pub trait TextEngine {
    /// Bumped by every mutation of the face set. A layout produced under a
    /// different generation is stale.
    fn font_generation(&self) -> FontGeneration;

    /// Shapes and lays out one source. `base` applies wherever
    /// [`TextSource::spans`] leaves a gap.
    fn layout(
        &mut self,
        kind: TextKind,
        source: &TextSource,
        base: &TextStyle,
        constraints: &TextConstraints,
        counters: &mut TextWorkCounters,
    ) -> TextLayout;
}
