#[path = "../support/mod.rs"]
mod support;

mod block_objects;
#[cfg(feature = "parity")]
mod corpus_is_wellformed;
mod edit_session_storage;
mod editable_text;
mod emphasis;
mod font_system_is_hermetic_and_deterministic;
mod hit_test_and_caret_are_derivable_from_the_layout_ir;
mod inline_objects;
mod layout_engine;
#[cfg(feature = "parity")]
mod layout_matches_the_cosmic_reference_goldens;
mod line_decision;
mod line_decision_gate;
mod object_labels;
#[cfg(feature = "raster")]
mod raster;
mod ruby;
#[cfg(feature = "parity")]
mod shaping_matches_the_cosmic_reference_goldens;
mod shaping_pipeline;
#[cfg(feature = "parity")]
mod structural_diff_reports_every_compared_field;
mod text_ids_are_generational_and_stale_handles_are_rejected;
