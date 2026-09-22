//! Nana-native text IR, font layer, shaper and layout engine, plus the
//! migration parity contract.
//!
//! This crate owns the *vocabulary* the NanaUI text migration is measured in:
//! the immutable layout IR, the stable generational handles, the caret and
//! hit-test derivations that run on top of them, and the structural diff plus
//! corpus that a new engine has to satisfy. Since Phase 1 (#90) it also owns
//! the font layer ([`font`]): registration, generations, face matching,
//! variation coordinates and coverage-driven fallback; since Phase 2 (#91),
//! shaping ([`shaping`]): segmentation, BiDi levels, HarfRust shaping and a
//! bounded shape cache; and since Phase 3 (#92), line layout ([`layout`]): the
//! single-line Label fast path, wrapping on UAX #14 opportunities and shaped
//! advances, per-line visual ordering, line box metrics, alignment, ellipsis
//! and a bounded layout cache.
//!
//! [`NativeTextEngine`] ties the three together behind [`TextEngine`]. Since
//! Phase 4 (#95) the UiWorld's retained text nodes can resolve through it and
//! keep their layouts behind [`TextLayoutStore`] handles. Since #99 this crate
//! is the only product text authority: Rust, NanaVue and Vue/CSS text is
//! measured by it, and `nana-ui`'s `NanaRenderer::text` draws the layouts the
//! Runtime retained.
//!
//! # Boundaries
//!
//! - **No `cosmic_text` or `cryoglyph` identifier appears under `src/`.** The
//!   cosmic reference engine that recorded the Phase 0 goldens was deleted in
//!   #99; the goldens stay, and the native engine is checked against them.
//!   `scripts/check-engine-boundary.py` keeps both engines out of every product
//!   crate's dependency graph.
//! - **Typography vocabulary is borrowed, not re-declared.** `TextStyle` and
//!   `TextConstraints` are built from `nana_ui_core`'s backend-neutral
//!   typography types so the eventual UiWorld adapter is a field-for-field move
//!   rather than a dozen hand-written enum conversions. Only these items may be
//!   named from `nana_ui_core`: `FontVariationSetting`, `FontKerningSpec`,
//!   `LineBreakSpec`, `FontFeatureSetting`, `LineHeightSpec`, `WordBreakSpec`,
//!   `TextWrapBreak`, `TextAlignSpec`, `DirSpec`, `WritingModeSpec`,
//!   `TextOrientationSpec`, `WritingContext`.
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
pub mod editable;
pub mod engine;
pub mod font;
pub mod id;
pub mod layout;
pub mod metrics;
pub mod parity;
pub mod shape;
pub mod shaping;
pub mod source;
pub mod style;

pub use constraints::{TextConstraints, TextScale};
pub use counters::TextWorkCounters;
pub use edit::{Affinity, CaretGeometry, CaretPosition, CaretStop, HitTestResult};
pub use editable::{
    CaretRect, Composition, EditChange, EditRevisions, EditSelection, EditSession, EditState,
    EditableText, EditorGeometry, Motion, TextEdit,
};
pub use engine::{
    NativeTextEngine, SharedTextEngine, TextEngine, TextEngineEpoch, lock_text_engine,
};
pub use id::{FontGeneration, FontId, FontSourceId, ShapeRunId, TextLayoutId, TextRevision};
pub use layout::{
    IntrinsicWidths, LayoutCacheBudget, LayoutCounters, LayoutRequest, Layouter, LineBox,
    LineBreakCause, OverflowFlags, StaleLayout, TextLayout, TextLayoutStore, TextRect,
};
pub use metrics::{LineMetrics, RunMetrics};
pub use shape::{GlyphFlags, RunDirection, RunOrientation, ScriptTag, ShapedGlyph, ShapedRun};
pub use source::{CompositionSegment, TextSource, TextSpan};
pub use style::{TextKind, TextStyle};
