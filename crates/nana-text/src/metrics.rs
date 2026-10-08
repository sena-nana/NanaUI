//! Vertical metrics, in physical px.
//!
//! `height_px` is the line advance (the CSS line box), which is a different
//! number from `ascent + descent`. Conflating the two is the classic source of
//! a one-off baseline shift that a screenshot catches and a metrics diff does
//! not, so they stay separate fields.

use serde::{Deserialize, Serialize};

/// Font metrics for one shaped run, scaled to the run's size.
///
/// The decoration fields come from the face's `post` (underline) and `OS/2`
/// (strikeout) tables, at the run's axis coordinates. A face without them
/// gets proportional stand-ins at read time, so a real run never reports a
/// zero thickness; zero means the metrics predate these fields (a golden
/// recorded before them), and a painter falls back on its own estimate.
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
pub struct RunMetrics {
    pub ascent_px: f32,
    pub descent_px: f32,
    pub line_gap_px: f32,
    /// Baseline to the **top** of the underline, positive **below** the
    /// baseline.
    #[serde(default)]
    pub underline_offset_px: f32,
    #[serde(default)]
    pub underline_thickness_px: f32,
    /// Baseline to the **top** of the strikeout, positive **above** the
    /// baseline.
    #[serde(default)]
    pub strikeout_offset_px: f32,
    #[serde(default)]
    pub strikeout_thickness_px: f32,
}

impl RunMetrics {
    /// Whether the decoration fields were read from a face (as opposed to
    /// deserialized from metrics that predate them).
    pub fn has_decorations(&self) -> bool {
        self.underline_thickness_px > 0.0 && self.strikeout_thickness_px > 0.0
    }
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
