//! Nana-native text IR and the migration parity contract. Not a text engine.
//!
//! This crate owns the *vocabulary* the NanaUI text migration is measured in:
//! the immutable layout IR, the stable generational handles, the caret and
//! hit-test derivations that run on top of them, and the structural diff plus
//! corpus that a new engine has to satisfy. Since Phase 1 (#90) it also owns
//! the font layer ([`font`]): registration, generations, face matching,
//! variation coordinates and coverage-driven fallback. It still ships no
//! shaping and no layout — those arrive in later phases.
//!
//! # Boundaries
//!
//! - **No `cosmic_text` or `cryoglyph` identifier appears under `src/`.** The
//!   cosmic reference engine lives in `tests/reference/`, reachable only from a
//!   dev dependency, so it can never enter a product dependency edge. It is
//!   temporary and is deleted along with that dependency once a native engine
//!   lands.
//! - **The product text path does not use this crate.** Product text still runs
//!   through `nana-ui`'s cosmic-text shaper and cryoglyph painter.
//! - **Typography vocabulary is borrowed, not re-declared.** `TextStyle` and
//!   `TextConstraints` are built from `nana_ui_core`'s backend-neutral
//!   typography types so the eventual UiWorld adapter is a field-for-field move
//!   rather than a dozen hand-written enum conversions. Only these items may be
//!   named from `nana_ui_core`: `FontVariationSetting`, `FontKerningSpec`,
//!   `LineBreakSpec`, `FontFeatureSetting`, `LineHeightSpec`, `WordBreakSpec`,
//!   `TextWrapBreak`, `DirSpec`, `WritingModeSpec`.
//!
//! # Staleness
//!
//! A [`TextLayout`] carries the [`TextRevision`] and [`FontGeneration`] it was
//! produced under, so deciding whether it can be reused is a comparison of two
//! integers rather than a re-fingerprint of the text. See
//! [`TextLayout::is_stale`].

pub mod constraints;
pub mod counters;
pub mod edit;
pub mod engine;
pub mod font;
pub mod id;
pub mod layout;
pub mod metrics;
pub mod parity;
pub mod shape;
pub mod source;
pub mod style;

pub use constraints::{TextConstraints, TextScale};
pub use counters::TextWorkCounters;
pub use edit::{Affinity, CaretGeometry, CaretPosition, HitTestResult};
pub use engine::TextEngine;
pub use id::{FontGeneration, FontId, FontSourceId, ShapeRunId, TextLayoutId, TextRevision};
pub use layout::{LineBox, LineBreakCause, OverflowFlags, TextLayout, TextRect};
pub use metrics::{LineMetrics, RunMetrics};
pub use shape::{GlyphFlags, RunDirection, ScriptTag, ShapedGlyph, ShapedRun};
pub use source::{CompositionSegment, TextSource, TextSpan};
pub use style::{TextKind, TextStyle};
