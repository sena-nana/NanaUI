//! Structural comparison of two layouts, and the corpus that drives it.
//!
//! This is the migration acceptance contract, and it outlived the engine it
//! was calibrated against: Phase 0 used [`compare`] to check the cosmic
//! reference against the committed goldens, and it is the same function that
//! now checks the native engine against those frozen goldens.
//!
//! Screenshots are not a substitute. A diff here names the line, the run, the
//! glyph and the field.

pub mod corpus;

mod diff;

pub use corpus::{
    CATEGORIES, CaseStatus, CorpusCaret, CorpusCase, CorpusError, CorpusHitTest,
    GOLDEN_SCHEMA_VERSION, Golden, KNOWN_FONT_FIXTURES, case_path, corpus_case_dir, corpus_dir,
    corpus_golden_dir, golden_ids, golden_path, load_case, load_cases, load_golden, write_golden,
};
pub use diff::{
    DEFAULT_TOLERANCES, DeltaScope, DeltaValue, LayoutDelta, ParityReport, Tolerances, compare,
    compare_golden, format_report,
};
