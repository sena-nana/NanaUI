//! The layout engine: immutable [`ShapedRun`](crate::ShapedRun)s in, an
//! immutable [`TextLayout`] out (Issue #92).
//!
//! ```text
//! ShapedText (#91) + TextConstraints
//!   → single-line Label fast path, or:
//!   → UAX #14 break opportunities over the paragraph text
//!   → greedy line breaking on shaped advances
//!   → rule L1/L2 visual ordering per line
//!   → line box metrics, alignment, ellipsis
//!   → immutable TextLayout, cached under a LayoutKey
//! ```
//!
//! **Shaping and layout are decoupled.** A layout reads the shaped runs and
//! copies the ones a line needs; it never reshapes. Changing the width, the
//! wrap mode, `max_lines` or the alignment re-runs *this* module only, which is
//! what makes a resize storm cost line work rather than shaping work.
//!
//! `unicode-linebreak` is named only from the private [`breaks`] module, the
//! same rule `harfrust` and `unicode-bidi` follow in
//! [`shaping`](crate::shaping).
//!
//! # What this module does not do
//!
//! - **`justify`**: `nana_ui_core::TextAlignSpec` has no justify keyword, so
//!   the product cannot ask for one. Deferred rather than half-built.
//! - **Vertical writing**: `vertical-rl` / `vertical-lr` (#59) need glyph
//!   orientation and vertical font metrics that neither this module nor the IR
//!   carries. A vertical request is laid out horizontally and says so:
//!   [`TextLayout::unsupported_writing_mode`] is set and
//!   [`LayoutCounters::vertical_writing_fallbacks`] counts it. Horizontal
//!   metrics are never reported as if they were vertical ones.
//! - **Caret and editing**: those are derivations on the finished IR
//!   ([`crate::edit`]), not engine state.

mod breaks;
mod cache;
mod engine;
mod ir;
mod key;
mod lines;

/// For the cross-list check in `source`'s tests: every one-byte separator that
/// ends a line has to fold. Layout itself reaches the list through `breaks`.
#[cfg(test)]
pub(crate) use breaks::FORCED_BREAKS;
pub use cache::LayoutCacheBudget;
pub use engine::{IntrinsicWidths, LayoutCounters, LayoutRequest, Layouter};
pub use ir::{LineBox, LineBreakCause, OverflowFlags, TextLayout, TextRect};
