//! OpenType / CSS typography subset stored on [`crate::LayoutStyle`].
//!
//! Honest: cosmic-text can apply feature tags and kerning, and can map `wght` /
//! `wdth` onto weight/stretch. Layout consumes `writing-mode: vertical-rl|lr`
//! for box axes; cosmic-text 0.19 has no vertical glyph orientation, so shaping
//! stays horizontal (not a rotated-box stand-in). Japanese `line-break:
//! strict|loose` are **not** applied.

use serde::{Deserialize, Serialize};
use std::hash::{Hash, Hasher};

/// One `font-variation-settings` axis (`"wght" 700`). `value` is the axis number.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct FontVariationSetting {
    pub tag: [u8; 4],
    pub value: f32,
}

impl Eq for FontVariationSetting {}

impl Hash for FontVariationSetting {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.tag.hash(state);
        self.value.to_bits().hash(state);
    }
}

/// CSS `font-kerning`. `None` disables the `kern` feature; `Auto`/`Normal` leave
/// the shaper default (typically on).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum FontKerningSpec {
    #[default]
    Auto,
    Normal,
    None,
}

/// CSS `line-break` subset. `loose` / `strict` are skipped at parse (no Japanese
/// line-breaking tables). `anywhere` is glyph wrap.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum LineBreakSpec {
    #[default]
    Auto,
    Normal,
    Anywhere,
}

impl FontVariationSetting {
    pub const WGHT: [u8; 4] = *b"wght";
    pub const WDTH: [u8; 4] = *b"wdth";

    pub const fn new(tag: [u8; 4], value: f32) -> Self {
        Self { tag, value }
    }

    pub fn wght_value(settings: &[Self]) -> Option<f32> {
        settings
            .iter()
            .rev()
            .find(|axis| axis.tag == Self::WGHT)
            .map(|axis| axis.value)
    }

    pub fn wdth_value(settings: &[Self]) -> Option<f32> {
        settings
            .iter()
            .rev()
            .find(|axis| axis.tag == Self::WDTH)
            .map(|axis| axis.value)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn variation_last_axis_wins() {
        let axes = [
            FontVariationSetting::new(*b"wght", 400.0),
            FontVariationSetting::new(*b"wdth", 125.0),
            FontVariationSetting::new(*b"wght", 700.0),
        ];
        assert_eq!(FontVariationSetting::wght_value(&axes), Some(700.0));
        assert_eq!(FontVariationSetting::wdth_value(&axes), Some(125.0));
        assert!(FontVariationSetting::wght_value(&[]).is_none());
    }
}
