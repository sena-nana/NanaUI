//! Semantic widget variants kept under the historical `widgets` path.
//!
//! The paint override and segmented-control inset that used to live here had
//! no consumers. Widget painting now consumes the semantic kinds from
//! `nana-ui-core` and the installed theme directly.
pub use nana_ui_core::{ButtonKind, CardKind};
