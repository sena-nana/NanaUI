//! Vertical metrics, in physical px.
//!
//! `height_px` is the line advance (the CSS line box), which is a different
//! number from `ascent + descent`. Conflating the two is the classic source of
//! a one-off baseline shift that a screenshot catches and a metrics diff does
//! not, so they stay separate fields.

use serde::{Deserialize, Serialize};

/// Font metrics for one shaped run, scaled to the run's size.
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
pub struct RunMetrics {
    pub ascent_px: f32,
    pub descent_px: f32,
    pub line_gap_px: f32,
}

/// Metrics for one laid-out line.
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
pub struct LineMetrics {
    /// Layout origin to the alphabetic baseline — the *central* baseline in
    /// a vertical layout, where it is the column's centre line.
    pub baseline_y_px: f32,
    /// Layout origin to the top of the line box.
    pub top_y_px: f32,
    /// Line advance. **Not** `ascent_px + descent_px`. The column's width in a
    /// vertical layout.
    pub height_px: f32,
    /// Largest ascent among the line's runs.
    pub ascent_px: f32,
    /// Largest descent among the line's runs.
    pub descent_px: f32,
    /// Inline extent of the line's content: its length down the column in a
    /// vertical layout.
    pub width_px: f32,
}
