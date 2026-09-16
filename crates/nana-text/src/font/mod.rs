//! The font layer: registration, generations, face matching, variation
//! coordinates and coverage-driven fallback (Issue #90).
//!
//! Output is a deterministic font choice for Phase 2 shaping to consume. This
//! module does not shape: [`FontSystem::resolve_text`] says which face each
//! cluster *should* use by cmap coverage, and shaping may still retry a
//! cluster that shapes to `.notdef`.
//!
//! `fontdb`, `skrifa` and `icu_properties` are implementation details of the
//! private `discovery`, `face` and `unicode` modules. Nothing public names
//! them, and `scripts/check-engine-boundary.py` keeps it that way.

mod coverage;
mod discovery;
mod face;
mod fallback;
mod matching;
mod query;
mod system;
mod unicode;
mod variations;

pub use coverage::{CoverageSet, DEFAULT_COVERAGE_BUDGET_BYTES};
pub use fallback::{FallbackPolicy, FontAssignment, FontChoiceReason, ScriptFallbackRule};
pub use query::{
    FamilyList, FamilyName, FontQuery, FontStretch, FontStyle, FontWeight, GenericFamily,
    LanguageTag,
};
pub use system::{
    FaceDescription, FaceDescriptor, FamilyResolution, FontBlob, FontCounters, FontData, FontError,
    FontOrigin, FontRegistration, FontSelection, FontSystem, font_blob,
};
pub use variations::{
    AxisCoord, FeatureValue, FontAxis, FontFeatures, FontInstance, FontInstanceKey, FontVariations,
    NamedInstance, Synthesis,
};
