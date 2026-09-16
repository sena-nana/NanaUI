//! What the container asks of a layout. Not style, and not device pixels
//! policy — see [`TextScale`] for where the device scale stops.

use nana_ui_core::{DirSpec, LineBreakSpec, TextWrapBreak, WordBreakSpec, WritingModeSpec};
use serde::{Deserialize, Serialize};

/// Fractional device scale applied to text.
///
/// A [`TextLayout`](crate::TextLayout) is reported in **physical** px, i.e.
/// logical px multiplied by this. Physical-pixel *snapping* of glyph origins is
/// a painter decision and is deliberately absent from this IR.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct TextScale {
    pub px_per_logical: f32,
}

impl Default for TextScale {
    fn default() -> Self {
        Self {
            px_per_logical: 1.0,
        }
    }
}

/// Container constraints for one layout.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct TextConstraints {
    #[serde(default)]
    pub max_width_px: Option<f32>,
    #[serde(default)]
    pub max_height_px: Option<f32>,
    /// `None` disables wrapping outright; `Some(_)` picks the algorithm.
    ///
    /// One field rather than today's `wrap: bool` plus a separate break mode,
    /// which can disagree with each other.
    #[serde(default)]
    pub wrap: Option<TextWrapBreak>,
    #[serde(default)]
    pub word_break: WordBreakSpec,
    #[serde(default)]
    pub line_break: LineBreakSpec,
    #[serde(default)]
    pub max_lines: Option<u16>,
    #[serde(default)]
    pub ellipsis: bool,
    /// Keep explicit `\n` as line breaks instead of collapsing them.
    #[serde(default)]
    pub preserve_lines: bool,
    #[serde(default)]
    pub base_direction: DirSpec,
    #[serde(default)]
    pub writing_mode: WritingModeSpec,
    /// Tab expansion in spaces. Explicit because an unstated tab width is just
    /// whichever default the backend happens to carry.
    #[serde(default = "default_tab_width")]
    pub tab_width: u8,
    #[serde(default)]
    pub scale: TextScale,
}

const fn default_tab_width() -> u8 {
    8
}

impl Default for TextConstraints {
    fn default() -> Self {
        Self {
            max_width_px: None,
            max_height_px: None,
            wrap: None,
            word_break: WordBreakSpec::default(),
            line_break: LineBreakSpec::default(),
            max_lines: None,
            ellipsis: false,
            preserve_lines: false,
            base_direction: DirSpec::default(),
            writing_mode: WritingModeSpec::default(),
            // Not `u8::default()`. A zero tab width is not a tab width.
            tab_width: default_tab_width(),
            scale: TextScale::default(),
        }
    }
}

impl TextConstraints {
    pub fn wraps(&self) -> bool {
        self.wrap.is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_defaulted_constraint_does_not_wrap_and_keeps_the_declared_tab_width() {
        let parsed: TextConstraints = serde_json::from_str("{}").expect("empty object is valid");
        assert_eq!(
            parsed,
            TextConstraints::default(),
            "serde defaults and Default must not drift apart"
        );
        assert!(!parsed.wraps());
        assert_eq!(parsed.tab_width, 8, "an omitted tab width is 8, not 0");
        assert_eq!(parsed.scale.px_per_logical, 1.0);
    }
}
