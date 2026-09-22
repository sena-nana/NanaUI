//! OpenType / CSS typography subset stored on [`crate::LayoutStyle`].
//!
//! `nana-text` applies feature tags and kerning, and applies declared
//! variation axes that exist on the loaded face (`wght`, `wdth`, custom tags
//! such as `BEVL`); `wght` and `wdth` also steer which face is picked. Axes
//! missing from the face are skipped (not remapped onto `wght`). Vertical
//! writing modes shape upright and sideways runs per `text-orientation`.
//! Japanese `line-break: strict|loose` are **not** applied.

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
        // `-0.0 == 0.0`, so they must hash alike.
        let value = if self.value == 0.0 {
            0.0f32
        } else {
            self.value
        };
        value.to_bits().hash(state);
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

    #[test]
    fn equal_settings_hash_alike() {
        use std::collections::hash_map::DefaultHasher;
        let hash = |setting: FontVariationSetting| {
            let mut hasher = DefaultHasher::new();
            setting.hash(&mut hasher);
            hasher.finish()
        };
        let positive = FontVariationSetting::new(*b"slnt", 0.0);
        let negative = FontVariationSetting::new(*b"slnt", -0.0);
        assert_eq!(positive, negative);
        assert_eq!(hash(positive), hash(negative));
    }
}
