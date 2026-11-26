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
    /// Font x-height in physical px (from OS/2 `sxHeight`, or `0.5 * ascent_px` fallback).
    #[serde(default)]
    pub x_height_px: f32,
    /// Font cap-height in physical px (from OS/2 `sCapHeight`, or `0.7 * ascent_px` fallback).
    #[serde(default)]
    pub cap_height_px: f32,
}

/// Measured vertical ink bounds of a laid-out line compared against its line
/// box centre, in layout coordinates (logical px, Y-down).
///
/// Useful for optically centring adjacent icons on a text heading's actual
/// glyph ink without shifting the text baseline itself.
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
pub struct VisualVerticalMetrics {
    /// Topmost ink edge of the line's glyphs, in layout px from the layout origin.
    pub ink_top_px: f32,
    /// Bottommost ink edge of the line's glyphs, in layout px from the layout origin.
    pub ink_bottom_px: f32,
    /// Midpoint of the glyph ink (`(ink_top_px + ink_bottom_px) * 0.5`).
    pub ink_center_y_px: f32,
    /// Midpoint of the line box (`line.top_y_px + line.height_px * 0.5`).
    pub line_center_y_px: f32,
    /// Signed Y offset from the line box centre to the glyph ink centre
    /// (`ink_center_y_px - line_center_y_px`; positive when ink sits lower
    /// than the centred line box).
    pub visual_center_offset_y_px: f32,
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
