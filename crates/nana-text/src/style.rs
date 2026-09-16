//! What the caller intends to do with a piece of text, and how it is typeset.
//! Not a widget, and not a CSS cascade — `nana-text` sits downstream of both.

use nana_ui_core::{FontFeatureSetting, FontKerningSpec, FontVariationSetting, LineHeightSpec};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

/// Which text path a source takes.
///
/// `Label` is the short fast path and never wraps or holds editor state.
/// `Paragraph` wraps and is hit-testable. `Editable` adds caret affinity and
/// IME composition. Keeping them apart is what stops a button label from
/// dragging an editor state machine behind it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum TextKind {
    #[default]
    Label,
    Paragraph,
    Editable,
}

/// Resolved typographic inputs for one span of text.
///
/// Every field is already resolved: there is no `inherit`, no family list and
/// no `normal` keyword. The cascade ran upstream.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TextStyle {
    /// `None` selects the host's default UI family. This is one family, not a
    /// CSS `font-family` list — fallback is the engine's job, not the caller's.
    #[serde(default)]
    pub font_family: Option<Arc<str>>,
    pub font_size_px: f32,
    /// Already resolved to a number. `nana-text` never sees `font-weight: bold`.
    pub font_weight: u16,
    #[serde(default)]
    pub italic: bool,
    #[serde(default)]
    pub line_height: Option<LineHeightSpec>,
    #[serde(default)]
    pub letter_spacing_px: f32,
    #[serde(default)]
    pub features: Vec<FontFeatureSetting>,
    #[serde(default)]
    pub variations: Vec<FontVariationSetting>,
    #[serde(default)]
    pub kerning: FontKerningSpec,
}

impl Default for TextStyle {
    fn default() -> Self {
        Self {
            font_family: None,
            font_size_px: 16.0,
            font_weight: 400,
            italic: false,
            line_height: None,
            letter_spacing_px: 0.0,
            features: Vec::new(),
            variations: Vec::new(),
            kerning: FontKerningSpec::default(),
        }
    }
}

impl TextStyle {
    /// The line-box height this style asks for, in the same px space as
    /// [`Self::font_size_px`]. `None` line-height means the engine's own
    /// default, which this does **not** guess at — callers that need a number
    /// before shaping use `nana_ui_core::text_line_box_height_px`.
    pub fn line_height_px(&self) -> Option<f32> {
        self.line_height
            .map(|spec| spec.resolve_px(self.font_size_px))
    }
}
