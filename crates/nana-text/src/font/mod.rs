//! The font layer: registration, generations, face matching, variation
//! coordinates and coverage-driven fallback (Issue #90).
//!
//! Output is a deterministic font choice for Phase 2 shaping to consume. This
//! module does not shape: [`FontSystem::resolve_text`] says which face each
//! cluster *should* use by cmap coverage, and shaping may still retry a
//! cluster that shapes to `.notdef`.
//!
//! `fontdb` and `skrifa` are implementation details of the private
//! `discovery` and `face` modules. Nothing public names them, and
//! `scripts/check-engine-boundary.py` keeps it that way.

mod coverage;
mod discovery;
mod face;
#[cfg(all(test, feature = "raster"))]
pub(crate) use face::map_char_for_test;
#[cfg(feature = "svg")]
pub(crate) use face::read_svg_glyph;
pub(crate) use face::{FaceOutlineSink, FaceScalerCache, scale_plain_outline};
#[cfg(feature = "raster")]
pub(crate) use face::{
    ColorBrush, ColorCompositeMode, ColorExtend, ColorGlyphOutline, ColorOutlineVerb, ColorPaintOp,
    ColorPaintRecording, ColorStopPaint, ColorTransform, RawBitmapPayload, read_color_bitmap,
    read_raster_caps, record_color_v1_glyph, scale_color_outline,
};
mod fallback;
mod mapped;
mod matching;
#[cfg(any(test, target_env = "ohos"))]
mod ohos;
mod query;
mod system;
mod ucd_tables;
pub(crate) mod unicode;
mod variations;

pub use coverage::{CoverageSet, DEFAULT_COVERAGE_BUDGET_BYTES};
pub use fallback::{FallbackPolicy, FontAssignment, FontChoiceReason, ScriptFallbackRule};
pub(crate) use query::canonical_bits as canonical_f32_bits;
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
