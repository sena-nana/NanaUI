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
        Self::axis_value(settings, Self::WGHT)
    }

    pub fn wdth_value(settings: &[Self]) -> Option<f32> {
        Self::axis_value(settings, Self::WDTH)
    }

    /// The value `settings` gives axis `tag`: the last declaration wins.
    pub fn axis_value(settings: &[Self], tag: [u8; 4]) -> Option<f32> {
        settings
            .iter()
            .rev()
            .find(|axis| axis.tag == tag)
            .map(|axis| axis.value)
    }

    /// Sets axis `tag` to `value` in place: the last declaration of the axis
    /// takes the value and earlier ones go, so the list still says the same
    /// thing about every other axis. An axis the list did not declare is
    /// appended.
    pub fn set_axis(settings: &mut Vec<Self>, tag: [u8; 4], value: f32) {
        match settings.iter().rposition(|axis| axis.tag == tag) {
            Some(last) => {
                settings[last].value = value;
                let mut index = 0;
                settings.retain(|axis| {
                    let keep = axis.tag != tag || index == last;
                    index += 1;
                    keep
                });
            }
            None => settings.push(Self::new(tag, value)),
        }
    }

    /// One declaration per axis (the last one), in tag order: the form two
    /// values are compared and interpolated in.
    pub fn normalized(settings: &[Self]) -> Vec<Self> {
        let mut out: Vec<Self> = Vec::with_capacity(settings.len());
        for axis in settings.iter().rev() {
            if !out.iter().any(|seen| seen.tag == axis.tag) {
                out.push(*axis);
            }
        }
        out.sort_by_key(|axis| axis.tag);
        out
    }

    /// Whether two [`Self::normalized`] lists name the same axes.
    pub fn same_axes(a: &[Self], b: &[Self]) -> bool {
        a.len() == b.len() && a.iter().zip(b).all(|(a, b)| a.tag == b.tag)
    }

    /// `(tag, from, to)` for every axis, when `from` and `to` declare the same
    /// axes; `None` when they do not.
    ///
    /// A `font-variation-settings` value interpolates axis by axis, and only
    /// between lists that name the same axes. An axis one side leaves out has
    /// no value to start or end at — its default is the face's, which no style
    /// knows — so such a pair is not interpolable and changes discretely.
    pub fn interpolable_pairs(from: &[Self], to: &[Self]) -> Option<Vec<([u8; 4], f32, f32)>> {
        let from = Self::normalized(from);
        let to = Self::normalized(to);
        if !Self::same_axes(&from, &to) {
            return None;
        }
        Some(
            from.iter()
                .zip(&to)
                .map(|(a, b)| (a.tag, a.value, b.value))
                .collect(),
        )
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
    fn set_axis_rewrites_the_winning_declaration_only() {
        let mut axes = vec![
            FontVariationSetting::new(*b"BEVL", 10.0),
            FontVariationSetting::new(*b"wdth", 125.0),
            FontVariationSetting::new(*b"BEVL", 20.0),
        ];
        FontVariationSetting::set_axis(&mut axes, *b"BEVL", 42.0);
        assert_eq!(
            axes,
            vec![
                FontVariationSetting::new(*b"wdth", 125.0),
                FontVariationSetting::new(*b"BEVL", 42.0),
            ]
        );
        FontVariationSetting::set_axis(&mut axes, *b"opsz", 12.0);
        assert_eq!(
            FontVariationSetting::axis_value(&axes, *b"opsz"),
            Some(12.0)
        );
        assert_eq!(
            FontVariationSetting::axis_value(&axes, *b"wdth"),
            Some(125.0)
        );
    }

    #[test]
    fn only_lists_naming_the_same_axes_interpolate() {
        let from = [
            FontVariationSetting::new(*b"wdth", 75.0),
            FontVariationSetting::new(*b"BEVL", 0.0),
            FontVariationSetting::new(*b"wdth", 80.0),
        ];
        let to = [
            FontVariationSetting::new(*b"BEVL", 100.0),
            FontVariationSetting::new(*b"wdth", 125.0),
        ];
        assert_eq!(
            FontVariationSetting::interpolable_pairs(&from, &to),
            Some(vec![(*b"BEVL", 0.0, 100.0), (*b"wdth", 80.0, 125.0)])
        );
        // `normal` against an axis list, and two different axis sets, have no
        // shared axes to interpolate.
        assert_eq!(FontVariationSetting::interpolable_pairs(&[], &to), None);
        assert_eq!(
            FontVariationSetting::interpolable_pairs(
                &[FontVariationSetting::new(*b"wght", 400.0)],
                &[FontVariationSetting::new(*b"wdth", 100.0)],
            ),
            None
        );
        assert_eq!(
            FontVariationSetting::interpolable_pairs(&[], &[]),
            Some(vec![])
        );
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
