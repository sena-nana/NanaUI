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
//! # Vertical writing
//!
//! A vertical line is laid out exactly like a horizontal one, in line-relative
//! coordinates: shaping already split it by UAX #50 into upright runs (shaped
//! top-to-bottom, with the font's vertical metrics) and sideways runs (shaped
//! as horizontal text, drawn rotated), and gave every glyph its advance along
//! the line. Line breaking, alignment and truncation run unchanged on those
//! advances. Only three things know the page is turned: which box dimension
//! budgets a line and which the stack ([`TextConstraints::inline_budget_px`](crate::TextConstraints::inline_budget_px)),
//! the baseline (central, not alphabetic) and the final mapping to the page
//! ([`TextLayout::physical_x_of_block`]).
//!
//! `unicode-linebreak` is named only from the private [`breaks`] module, the
//! same rule `harfrust` and `unicode-bidi` follow in
//! [`shaping`](crate::shaping).
//!
//! # What this module does not do
//!
//! - **`justify`**: `nana_ui_core::TextAlignSpec` has no justify keyword, so
//!   the product cannot ask for one. Deferred rather than half-built.
//! - **Vertical editing**: `vertical-rl` / `vertical-lr` lay out as columns
//!   (see *Vertical writing* below) for every kind of text but
//!   [`TextKind::Editable`](crate::TextKind::Editable), whose caret movement and
//!   selection across columns are not built. An editor asked for vertical
//!   text is laid out horizontally and says so:
//!   [`TextLayout::unsupported_writing_mode`] is set and
//!   [`LayoutCounters::vertical_writing_fallbacks`] counts it.
//!   `sideways-*` and `text-orientation` never reach this module: the box
//!   layout refuses them before a text node sees them.
//! - **Caret and editing**: those are derivations on the finished IR
//!   ([`crate::edit`]), not engine state.

mod breaks;
mod cache;
mod engine;
mod ir;
mod key;
mod lines;
mod store;

/// For the cross-list check in `source`'s tests: every one-byte separator that
/// ends a line has to fold. Layout itself reaches the list through `breaks`.
#[cfg(test)]
pub(crate) use breaks::FORCED_BREAKS;
pub use cache::LayoutCacheBudget;
pub use engine::{IntrinsicWidths, LayoutCounters, LayoutRequest, Layouter};
pub use ir::{LineBox, LineBreakCause, OverflowFlags, TextLayout, TextRect};
pub use store::{StaleLayout, TextLayoutStore};
