//! What text asks the font system for. Not a face, and not a CSS cascade.
//!
//! CSS `font-family` / `font-weight` / `font-stretch` / `font-style`, a Rust
//! [`TextStyle`], and NanaVue props all end as one [`FontQuery`]. It is `Eq +
//! Hash` so the font system can cache selections on it directly.

use crate::style::TextStyle;
use nana_ui_core::FontVariationSetting;
use serde::{Deserialize, Serialize};
use std::hash::{Hash, Hasher};
use std::sync::Arc;

/// Bit pattern of an `f32` for `Eq` / `Hash`, with `-0.0` folded into `0.0`
/// so two numerically equal queries can never land in different cache slots.
pub(crate) fn canonical_bits(value: f32) -> u32 {
    if value == 0.0 { 0 } else { value.to_bits() }
}

/// A CSS generic family.
///
/// Which concrete families each one names is policy, owned by
/// [`FallbackPolicy`](super::FallbackPolicy), not by this enum.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum GenericFamily {
    Serif,
    SansSerif,
    Monospace,
    Cursive,
    Fantasy,
    SystemUi,
    Emoji,
    Math,
}

impl GenericFamily {
    /// The unquoted CSS keyword, if `keyword` is one. `ui-*` spellings fold
    /// onto their plain generic.
    pub fn from_css_keyword(keyword: &str) -> Option<Self> {
        let generic = match keyword.to_ascii_lowercase().as_str() {
            "serif" | "ui-serif" => Self::Serif,
            "sans-serif" | "ui-sans-serif" => Self::SansSerif,
            "monospace" | "ui-monospace" => Self::Monospace,
            "cursive" => Self::Cursive,
            "fantasy" => Self::Fantasy,
            "system-ui" => Self::SystemUi,
            "emoji" => Self::Emoji,
            "math" => Self::Math,
            _ => return None,
        };
        Some(generic)
    }
}

/// One entry of a family list.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum FamilyName {
    /// A concrete family, matched ASCII-case-insensitively against every name
    /// a face carries (typographic, legacy and localized).
    Named(Arc<str>),
    Generic(GenericFamily),
}

impl FamilyName {
    pub fn named(name: &str) -> Self {
        Self::Named(Arc::from(name))
    }
}

/// An ordered family preference list: `font-family: "Inter", sans-serif`.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub struct FamilyList(Vec<FamilyName>);

impl FamilyList {
    pub fn new(families: Vec<FamilyName>) -> Self {
        Self(families)
    }

    pub fn as_slice(&self) -> &[FamilyName] {
        &self.0
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// Parses a CSS `font-family` value.
    ///
    /// Quoted entries are always names, so `"serif"` in quotes is a family
    /// called serif. Unquoted entries are generic keywords when they spell one,
    /// otherwise their identifiers joined by single spaces. Empty entries are
    /// dropped rather than turned into a family named "".
    pub fn parse_css(value: &str) -> Self {
        let mut families = Vec::new();
        for entry in split_css_list(value) {
            match entry {
                CssEntry::Quoted(name) => {
                    if !name.is_empty() {
                        families.push(FamilyName::Named(Arc::from(name.as_str())));
                    }
                }
                CssEntry::Bare(words) => {
                    let joined = words.split_whitespace().collect::<Vec<_>>().join(" ");
                    if joined.is_empty() {
                        continue;
                    }
                    families.push(match GenericFamily::from_css_keyword(&joined) {
                        Some(generic) => FamilyName::Generic(generic),
                        None => FamilyName::Named(Arc::from(joined.as_str())),
                    });
                }
            }
        }
        Self(families)
    }
}

enum CssEntry {
    Quoted(String),
    Bare(String),
}

fn split_css_list(value: &str) -> Vec<CssEntry> {
    let mut entries = Vec::new();
    let mut chars = value.chars().peekable();
    loop {
        while chars.peek().is_some_and(|c| c.is_whitespace()) {
            chars.next();
        }
        let Some(&first) = chars.peek() else {
            break;
        };
        if first == '"' || first == '\'' {
            chars.next();
            let mut name = String::new();
            while let Some(c) = chars.next() {
                if c == first {
                    break;
                }
                if c == '\\' {
                    if let Some(escaped) = chars.next() {
                        name.push(escaped);
                    }
                    continue;
                }
                name.push(c);
            }
            entries.push(CssEntry::Quoted(name));
            // Anything between the closing quote and the comma is malformed;
            // skip it rather than glue it onto the next entry.
            for c in chars.by_ref() {
                if c == ',' {
                    break;
                }
            }
        } else {
            let mut words = String::new();
            for c in chars.by_ref() {
                if c == ',' {
                    break;
                }
                words.push(c);
            }
            entries.push(CssEntry::Bare(words));
        }
    }
    entries
}

/// CSS `font-weight` on the 1..=1000 scale. Fractional, because a variable
/// `wght` coordinate is.
#[derive(Debug, Clone, Copy, PartialEq, PartialOrd, Serialize, Deserialize)]
pub struct FontWeight(pub f32);

impl FontWeight {
    pub const NORMAL: Self = Self(400.0);
    pub const BOLD: Self = Self(700.0);

    /// Clamped to the CSS range; a non-finite weight is `normal`.
    pub fn new(value: f32) -> Self {
        if value.is_finite() {
            Self(value.clamp(1.0, 1000.0))
        } else {
            Self::NORMAL
        }
    }
}

impl Default for FontWeight {
    fn default() -> Self {
        Self::NORMAL
    }
}

impl Eq for FontWeight {}

impl Hash for FontWeight {
    fn hash<H: Hasher>(&self, state: &mut H) {
        canonical_bits(self.0).hash(state);
    }
}

/// CSS `font-stretch` as a percentage of normal width (`wdth` units).
#[derive(Debug, Clone, Copy, PartialEq, PartialOrd, Serialize, Deserialize)]
pub struct FontStretch(pub f32);

impl FontStretch {
    pub const NORMAL: Self = Self(100.0);

    /// Clamped to CSS's 50%..=200%; a non-finite stretch is `normal`.
    pub fn new(percent: f32) -> Self {
        if percent.is_finite() {
            Self(percent.clamp(50.0, 200.0))
        } else {
            Self::NORMAL
        }
    }
}

impl Default for FontStretch {
    fn default() -> Self {
        Self::NORMAL
    }
}

impl Eq for FontStretch {}

impl Hash for FontStretch {
    fn hash<H: Hasher>(&self, state: &mut H) {
        canonical_bits(self.0).hash(state);
    }
}

/// CSS `font-style`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum FontStyle {
    #[default]
    Normal,
    Italic,
    /// `oblique` at CSS's default 14deg. An explicit angle is a `slnt`
    /// variation, not a second style keyword.
    Oblique,
}

/// A BCP 47 language tag, lower-cased with `-` separators.
///
/// Only used as a fallback hint (`zh-Hant` picks traditional Han faces, `ja`
/// Japanese ones); it is never validated against a registry.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct LanguageTag(Arc<str>);

impl LanguageTag {
    pub fn new(tag: &str) -> Option<Self> {
        let normalized = tag.trim().replace('_', "-").to_ascii_lowercase();
        if normalized.is_empty() {
            None
        } else {
            Some(Self(Arc::from(normalized.as_str())))
        }
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// True when `prefix` equals this tag or is a whole-subtag prefix of it:
    /// `zh-hant` matches `zh-hant-tw`, `zh` does not match `zhuang`.
    pub fn matches_prefix(&self, prefix: &str) -> bool {
        let prefix = prefix.to_ascii_lowercase();
        self.0.as_ref() == prefix
            || (self.0.starts_with(prefix.as_str()) && self.0.as_bytes()[prefix.len()] == b'-')
    }
}

/// Everything face selection depends on.
///
/// Variation settings and features are deliberately absent: they pick
/// coordinates *within* a face ([`FontInstance`](super::FontInstance)), not
/// which face, with the one exception documented on [`Self::from_style`].
#[derive(Debug, Clone, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub struct FontQuery {
    pub families: FamilyList,
    #[serde(default)]
    pub weight: FontWeight,
    #[serde(default)]
    pub stretch: FontStretch,
    #[serde(default)]
    pub style: FontStyle,
    #[serde(default)]
    pub language: Option<LanguageTag>,
}

impl FontQuery {
    /// The query a resolved [`TextStyle`] asks for.
    ///
    /// `font_family` is parsed as a CSS family list, and `sans-serif` is
    /// appended when the list names no generic, so an unmatched name still
    /// resolves to the host's UI family rather than to nothing.
    ///
    /// **Weight / stretch precedence.** An explicit `"wght"` in
    /// `font-variation-settings` wins over `font-weight`, and an explicit
    /// `"wdth"` wins over stretch, for face *selection* as well as for the
    /// axis coordinate. That is the product rule `nana-ui` already ships
    /// (`wght` merges into weight), kept so the migration does not change which
    /// face a page gets. Any other axis, including custom ones such as `BEVL`,
    /// never influences selection and never becomes `wght`.
    pub fn from_style(style: &TextStyle, language: Option<LanguageTag>) -> Self {
        let mut families = style
            .font_family
            .as_deref()
            .map(FamilyList::parse_css)
            .unwrap_or_default();
        if !families
            .as_slice()
            .iter()
            .any(|family| matches!(family, FamilyName::Generic(_)))
        {
            families
                .0
                .push(FamilyName::Generic(GenericFamily::SansSerif));
        }
        let weight = FontVariationSetting::wght_value(&style.variations)
            .filter(|value| value.is_finite())
            .unwrap_or(f32::from(style.font_weight));
        let stretch = FontVariationSetting::wdth_value(&style.variations)
            .filter(|value| value.is_finite())
            .unwrap_or(FontStretch::NORMAL.0);
        Self {
            families,
            weight: FontWeight::new(weight),
            stretch: FontStretch::new(stretch),
            style: if style.italic {
                FontStyle::Italic
            } else {
                FontStyle::Normal
            },
            language,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn css_family_lists_keep_quoted_keywords_as_names() {
        let list = FamilyList::parse_css(r#" "Noto Sans SC", 'serif' , Segoe   UI, sans-serif,, "#);
        assert_eq!(
            list.as_slice(),
            &[
                FamilyName::named("Noto Sans SC"),
                FamilyName::named("serif"),
                FamilyName::named("Segoe UI"),
                FamilyName::Generic(GenericFamily::SansSerif),
            ]
        );
    }

    #[test]
    fn an_explicit_wght_axis_drives_selection_weight_but_bevl_does_not() {
        let style = TextStyle {
            font_family: Some(Arc::from("Display")),
            font_weight: 400,
            variations: vec![
                FontVariationSetting::new(*b"BEVL", 900.0),
                FontVariationSetting::new(*b"wght", 650.0),
            ],
            ..TextStyle::default()
        };
        let query = FontQuery::from_style(&style, None);
        assert_eq!(query.weight, FontWeight(650.0));
        assert_eq!(
            query.families.as_slice().last(),
            Some(&FamilyName::Generic(GenericFamily::SansSerif))
        );

        let bevl_only = TextStyle {
            variations: vec![FontVariationSetting::new(*b"BEVL", 900.0)],
            ..style
        };
        assert_eq!(
            FontQuery::from_style(&bevl_only, None).weight,
            FontWeight(400.0),
            "a custom axis must never be read as a weight"
        );
    }

    #[test]
    fn negative_zero_and_zero_hash_to_the_same_query() {
        use std::collections::hash_map::DefaultHasher;
        let hash = |query: &FontQuery| {
            let mut hasher = DefaultHasher::new();
            query.hash(&mut hasher);
            hasher.finish()
        };
        let a = FontQuery {
            stretch: FontStretch(0.0),
            ..FontQuery::default()
        };
        let b = FontQuery {
            stretch: FontStretch(-0.0),
            ..FontQuery::default()
        };
        assert_eq!(a, b);
        assert_eq!(hash(&a), hash(&b));
    }

    #[test]
    fn language_prefixes_match_whole_subtags_only() {
        let tag = LanguageTag::new("zh_Hant_TW").unwrap();
        assert_eq!(tag.as_str(), "zh-hant-tw");
        assert!(tag.matches_prefix("zh"));
        assert!(tag.matches_prefix("zh-Hant"));
        assert!(!LanguageTag::new("zhuang").unwrap().matches_prefix("zh"));
    }
}
