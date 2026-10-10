//! OpenType / CSS typography subset stored on [`crate::LayoutStyle`].
//!
//! `nana-text` applies feature tags and kerning, and applies declared
//! variation axes that exist on the loaded face (`wght`, `wdth`, custom tags
//! such as `BEVL`); `wght` and `wdth` also steer which face is picked. Axes
//! missing from the face are skipped (not remapped onto `wght`). Vertical
//! writing modes shape upright and sideways runs per `text-orientation`.
//! `line-break: strict|loose`, `text-spacing-trim`, `text-autospace`,
//! `text-justify` and `text-wrap-style: pretty` are applied by `nana-text`'s
//! line decision (Issue #211).

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

/// CSS `line-break`. `strict` forbids more breaks before CJK small kana,
/// prolonged sound marks and the like than `normal`; `loose` allows more.
/// `anywhere` is glyph wrap.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum LineBreakSpec {
    #[default]
    Auto,
    Normal,
    Anywhere,
    Strict,
    Loose,
}

/// CSS `text-spacing-trim`: how much of the blank half of fullwidth CJK
/// punctuation a line keeps.
///
/// `SpaceAll` keeps every glyph's full advance. `Normal` closes up a pair of
/// adjacent punctuation marks and a closing mark at the end of a line that
/// would not fit otherwise. `TrimBoth` also closes up an opening mark at the
/// start of a line. `Auto` is `Normal`, and a line that runs short may close
/// up its other punctuation, at a cost, rather than break early.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum TextSpacingTrimSpec {
    #[default]
    SpaceAll,
    Normal,
    TrimBoth,
    Auto,
}

impl TextSpacingTrimSpec {
    /// Whether any punctuation closes up.
    pub const fn trims(self) -> bool {
        !matches!(self, Self::SpaceAll)
    }
}

/// CSS `text-autospace`: whether a gap goes between ideographs and Latin
/// letters or digits written against them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum TextAutospaceSpec {
    #[default]
    NoAutospace,
    Normal,
}

/// CSS `text-justify`: where a justified line puts its slack.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum TextJustifySpec {
    /// Between words, and between CJK characters.
    #[default]
    Auto,
    /// Nowhere: the line is laid out at its start.
    None,
    InterWord,
    InterCharacter,
}

/// The CJK-facing typography a box declares: each `None` inherits.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub struct TextTypography {
    #[serde(default)]
    pub spacing_trim: Option<TextSpacingTrimSpec>,
    #[serde(default)]
    pub autospace: Option<TextAutospaceSpec>,
    #[serde(default)]
    pub justify: Option<TextJustifySpec>,
    #[serde(default)]
    pub wrap_style: Option<TextWrapStyleSpec>,
    /// Whether `text-align` is `justify`: inherited as `text-align` is, so a
    /// paragraph under a justified box justifies its lines.
    #[serde(default)]
    pub justify_lines: Option<bool>,
}

impl TextTypography {
    /// Declares nothing: every property inherits.
    pub const INHERIT: Self = Self {
        spacing_trim: None,
        autospace: None,
        justify: None,
        wrap_style: None,
        justify_lines: None,
    };

    /// What a box declares, else what it inherits, property by property.
    pub fn inherit_from(self, parent: Self) -> Self {
        Self {
            spacing_trim: self.spacing_trim.or(parent.spacing_trim),
            autospace: self.autospace.or(parent.autospace),
            justify: self.justify.or(parent.justify),
            wrap_style: self.wrap_style.or(parent.wrap_style),
            justify_lines: self.justify_lines.or(parent.justify_lines),
        }
    }

    /// The values text is laid out with. `cjk` is whether the text's
    /// language is Chinese or Japanese: there an unset `text-spacing-trim`
    /// is `normal` (punctuation closes up at fixed amounts, no cost search);
    /// elsewhere it is `space-all`.
    pub fn used(self, cjk: bool) -> UsedTextTypography {
        UsedTextTypography {
            spacing_trim: self.spacing_trim.unwrap_or(if cjk {
                TextSpacingTrimSpec::Normal
            } else {
                TextSpacingTrimSpec::SpaceAll
            }),
            autospace: self.autospace.unwrap_or_default(),
            justify: self.justify.unwrap_or_default(),
            wrap_style: self.wrap_style.unwrap_or_default(),
            justify_lines: self.justify_lines.unwrap_or(false),
        }
    }
}

/// [`TextTypography`] with every property decided: what the text engine is
/// handed, and what a renderer laying the text out again must hand it too.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub struct UsedTextTypography {
    pub spacing_trim: TextSpacingTrimSpec,
    pub autospace: TextAutospaceSpec,
    pub justify: TextJustifySpec,
    pub wrap_style: TextWrapStyleSpec,
    pub justify_lines: bool,
}

/// CSS `text-wrap-style`. `Pretty` lets a paragraph look ahead a bounded
/// number of lines when it chooses where to break.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum TextWrapStyleSpec {
    #[default]
    Auto,
    Pretty,
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
