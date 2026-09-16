//! Which families to try when the requested ones do not cover a cluster, and
//! why a cluster ended up on the face it did.
//!
//! Two layers, as #90 specifies:
//!
//! ```text
//! policy fallback candidates   (this file: ordered family names)
//!         ↓
//! coverage probe               (FontSystem::resolve_text: cmap coverage cache)
//!         ↓
//! shaping retry                (Phase 2: a covered cluster can still shape to
//!                               .notdef, e.g. a missing GSUB ligature)
//! ```
//!
//! The policy is plain data, so a hermetic test builds exactly the chain it
//! asserts on, and a host replaces the platform default wholesale rather than
//! patching around it.

use super::query::{GenericFamily, LanguageTag};
use crate::id::FontId;
use crate::shape::ScriptTag;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::ops::Range;
use std::sync::Arc;

/// Script-specific fallback families, optionally narrowed to a language.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ScriptFallbackRule {
    pub script: ScriptTag,
    /// Whole-subtag prefix, e.g. `ja` or `zh-hant`. `None` applies to any
    /// language and is tried after every language-specific rule.
    pub language: Option<Arc<str>>,
    pub families: Vec<Arc<str>>,
}

/// Ordered family names for each fallback situation.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct FallbackPolicy {
    generic: BTreeMap<GenericFamily, Vec<Arc<str>>>,
    scripts: Vec<ScriptFallbackRule>,
    /// List colour families before monochrome ones.
    emoji: Vec<Arc<str>>,
    symbol: Vec<Arc<str>>,
    last_resort: Vec<Arc<str>>,
}

fn names<'a>(families: impl IntoIterator<Item = &'a str>) -> Vec<Arc<str>> {
    families.into_iter().map(Arc::from).collect()
}

impl FallbackPolicy {
    /// No families at all. What hermetic tests start from.
    pub fn empty() -> Self {
        Self::default()
    }

    pub fn set_generic<'a>(
        &mut self,
        generic: GenericFamily,
        families: impl IntoIterator<Item = &'a str>,
    ) -> &mut Self {
        self.generic.insert(generic, names(families));
        self
    }

    /// Families a generic resolves to. `system-ui` with no entry of its own
    /// uses `sans-serif`; `emoji` with no entry uses the emoji fallback list.
    pub fn generic(&self, generic: GenericFamily) -> &[Arc<str>] {
        match self.generic.get(&generic) {
            Some(families) => families,
            None => match generic {
                GenericFamily::SystemUi => self.generic(GenericFamily::SansSerif),
                GenericFamily::Emoji => &self.emoji,
                _ => &[],
            },
        }
    }

    pub fn push_script_rule<'a>(
        &mut self,
        script: ScriptTag,
        language: Option<&str>,
        families: impl IntoIterator<Item = &'a str>,
    ) -> &mut Self {
        self.scripts.push(ScriptFallbackRule {
            script,
            language: language.map(|tag| Arc::from(tag.to_ascii_lowercase().as_str())),
            families: names(families),
        });
        self
    }

    pub fn set_emoji_families<'a>(
        &mut self,
        families: impl IntoIterator<Item = &'a str>,
    ) -> &mut Self {
        self.emoji = names(families);
        self
    }

    pub fn set_symbol_families<'a>(
        &mut self,
        families: impl IntoIterator<Item = &'a str>,
    ) -> &mut Self {
        self.symbol = names(families);
        self
    }

    pub fn set_last_resort_families<'a>(
        &mut self,
        families: impl IntoIterator<Item = &'a str>,
    ) -> &mut Self {
        self.last_resort = names(families);
        self
    }

    pub fn emoji_families(&self) -> &[Arc<str>] {
        &self.emoji
    }

    pub fn symbol_families(&self) -> &[Arc<str>] {
        &self.symbol
    }

    pub fn last_resort_families(&self) -> &[Arc<str>] {
        &self.last_resort
    }

    /// Families for `script`: rules whose language matches the hint first, in
    /// rule order, then language-agnostic rules, in rule order.
    pub fn script_families(
        &self,
        script: ScriptTag,
        language: Option<&LanguageTag>,
    ) -> Vec<Arc<str>> {
        let matching = |rule: &&ScriptFallbackRule| rule.script == script;
        let specific =
            self.scripts
                .iter()
                .filter(matching)
                .filter(|rule| match (&rule.language, language) {
                    (Some(prefix), Some(language)) => language.matches_prefix(prefix),
                    _ => false,
                });
        let general = self
            .scripts
            .iter()
            .filter(matching)
            .filter(|rule| rule.language.is_none());
        let mut families: Vec<Arc<str>> = Vec::new();
        for family in specific
            .chain(general)
            .flat_map(|rule| rule.families.iter())
        {
            if !families
                .iter()
                .any(|known| known.eq_ignore_ascii_case(family))
            {
                families.push(Arc::clone(family));
            }
        }
        families
    }

    /// The platform's usual families. Missing families are harmless: a name
    /// that resolves to no face is skipped.
    pub fn platform_default() -> Self {
        let mut policy = Self::empty();
        platform::fill(&mut policy);
        policy
    }
}

const HANI: ScriptTag = ScriptTag(*b"Hani");
const HIRA: ScriptTag = ScriptTag(*b"Hira");
const KANA: ScriptTag = ScriptTag(*b"Kana");
const HANG: ScriptTag = ScriptTag(*b"Hang");
const ARAB: ScriptTag = ScriptTag(*b"Arab");
const HEBR: ScriptTag = ScriptTag(*b"Hebr");
const THAI: ScriptTag = ScriptTag(*b"Thai");
const DEVA: ScriptTag = ScriptTag(*b"Deva");

struct PlatformFamilies {
    sans: &'static [&'static str],
    serif: &'static [&'static str],
    mono: &'static [&'static str],
    cursive: &'static [&'static str],
    fantasy: &'static [&'static str],
    math: &'static [&'static str],
    han_sc: &'static [&'static str],
    han_tc: &'static [&'static str],
    han_hk: &'static [&'static str],
    japanese: &'static [&'static str],
    korean: &'static [&'static str],
    arabic: &'static [&'static str],
    hebrew: &'static [&'static str],
    thai: &'static [&'static str],
    devanagari: &'static [&'static str],
    emoji: &'static [&'static str],
    symbol: &'static [&'static str],
}

impl PlatformFamilies {
    fn apply(&self, policy: &mut FallbackPolicy) {
        policy
            .set_generic(GenericFamily::SansSerif, self.sans.iter().copied())
            .set_generic(GenericFamily::Serif, self.serif.iter().copied())
            .set_generic(GenericFamily::Monospace, self.mono.iter().copied())
            .set_generic(GenericFamily::Cursive, self.cursive.iter().copied())
            .set_generic(GenericFamily::Fantasy, self.fantasy.iter().copied())
            .set_generic(GenericFamily::Math, self.math.iter().copied())
            .set_emoji_families(self.emoji.iter().copied())
            .set_symbol_families(self.symbol.iter().copied())
            .set_last_resort_families(self.symbol.iter().chain(self.emoji).copied());
        let japanese_first: Vec<&str> = self.japanese.iter().chain(self.han_sc).copied().collect();
        policy
            .push_script_rule(HANI, Some("ja"), japanese_first.iter().copied())
            .push_script_rule(
                HANI,
                Some("ko"),
                self.korean.iter().chain(self.han_tc).copied(),
            )
            .push_script_rule(
                HANI,
                Some("zh-hk"),
                self.han_hk.iter().chain(self.han_tc).copied(),
            )
            .push_script_rule(
                HANI,
                Some("zh-mo"),
                self.han_hk.iter().chain(self.han_tc).copied(),
            )
            .push_script_rule(HANI, Some("zh-hant"), self.han_tc.iter().copied())
            .push_script_rule(HANI, Some("zh-tw"), self.han_tc.iter().copied())
            .push_script_rule(HANI, None, self.han_sc.iter().chain(self.japanese).copied())
            .push_script_rule(HIRA, None, japanese_first.iter().copied())
            .push_script_rule(KANA, None, japanese_first.iter().copied())
            .push_script_rule(HANG, None, self.korean.iter().copied())
            .push_script_rule(ARAB, None, self.arabic.iter().copied())
            .push_script_rule(HEBR, None, self.hebrew.iter().copied())
            .push_script_rule(THAI, None, self.thai.iter().copied())
            .push_script_rule(DEVA, None, self.devanagari.iter().copied());
    }
}

#[cfg(target_os = "windows")]
mod platform {
    pub(super) fn fill(policy: &mut super::FallbackPolicy) {
        super::PlatformFamilies {
            sans: &["Segoe UI", "Microsoft YaHei UI"],
            serif: &["Times New Roman", "SimSun"],
            mono: &["Cascadia Mono", "Consolas"],
            cursive: &["Comic Sans MS"],
            fantasy: &["Impact"],
            math: &["Cambria Math"],
            han_sc: &["Microsoft YaHei UI", "Microsoft YaHei", "SimSun"],
            han_tc: &["Microsoft JhengHei UI", "Microsoft JhengHei", "MingLiU"],
            han_hk: &["Microsoft JhengHei UI", "MingLiU_HKSCS"],
            japanese: &["Yu Gothic UI", "Meiryo UI", "MS Gothic"],
            korean: &["Malgun Gothic", "Gulim"],
            arabic: &["Segoe UI"],
            hebrew: &["Segoe UI"],
            thai: &["Leelawadee UI"],
            devanagari: &["Nirmala UI"],
            emoji: &["Segoe UI Emoji"],
            symbol: &["Segoe UI Symbol", "Segoe UI Historic"],
        }
        .apply(policy);
    }
}

#[cfg(any(target_os = "macos", target_os = "ios"))]
mod platform {
    pub(super) fn fill(policy: &mut super::FallbackPolicy) {
        super::PlatformFamilies {
            sans: &["Helvetica Neue", "PingFang SC"],
            serif: &["Times", "Songti SC"],
            mono: &["Menlo", "SF Mono"],
            cursive: &["Apple Chancery"],
            fantasy: &["Papyrus"],
            math: &["STIX Two Math"],
            han_sc: &["PingFang SC", "Hiragino Sans GB"],
            han_tc: &["PingFang TC"],
            han_hk: &["PingFang HK"],
            japanese: &["Hiragino Sans", "Hiragino Kaku Gothic ProN"],
            korean: &["Apple SD Gothic Neo"],
            arabic: &["Geeza Pro"],
            hebrew: &["Arial Hebrew"],
            thai: &["Thonburi"],
            devanagari: &["Kohinoor Devanagari"],
            emoji: &["Apple Color Emoji"],
            symbol: &["Apple Symbols"],
        }
        .apply(policy);
    }
}

#[cfg(not(any(target_os = "windows", target_os = "macos", target_os = "ios")))]
mod platform {
    pub(super) fn fill(policy: &mut super::FallbackPolicy) {
        super::PlatformFamilies {
            sans: &["Noto Sans", "Roboto", "DejaVu Sans"],
            serif: &["Noto Serif", "DejaVu Serif"],
            mono: &["Noto Sans Mono", "DejaVu Sans Mono"],
            cursive: &["Noto Sans"],
            fantasy: &["Noto Sans"],
            math: &["Noto Sans Math"],
            han_sc: &["Noto Sans CJK SC", "Noto Sans SC"],
            han_tc: &["Noto Sans CJK TC", "Noto Sans TC"],
            han_hk: &["Noto Sans CJK HK", "Noto Sans HK"],
            japanese: &["Noto Sans CJK JP", "Noto Sans JP"],
            korean: &["Noto Sans CJK KR", "Noto Sans KR"],
            arabic: &["Noto Sans Arabic", "Noto Naskh Arabic"],
            hebrew: &["Noto Sans Hebrew"],
            thai: &["Noto Sans Thai"],
            devanagari: &["Noto Sans Devanagari"],
            emoji: &["Noto Color Emoji", "Noto Emoji"],
            symbol: &["Noto Sans Symbols", "Noto Sans Symbols 2"],
        }
        .apply(policy);
    }
}

/// Why a cluster landed on the face it did.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case", tag = "kind")]
pub enum FontChoiceReason {
    /// The selection's primary face covers it.
    Primary,
    /// Entry `index` of the selection's fallback chain (0 = the first face
    /// after the primary) covers it.
    FamilyChain {
        index: u16,
    },
    /// The emoji policy family `family` covers it.
    EmojiPolicy {
        family: Arc<str>,
    },
    /// The script policy for `script` (under the language hint, if any) named
    /// `family`, which covers it.
    ScriptPolicy {
        script: ScriptTag,
        family: Arc<str>,
    },
    /// A cluster with no specific script, covered by symbol policy `family`.
    SymbolPolicy {
        family: Arc<str>,
    },
    LastResort {
        family: Arc<str>,
    },
    /// Only default-ignorable codepoints (ZWJ, variation selectors); it rides
    /// on the preceding cluster's face.
    Ignorable,
    /// No candidate covers it. Shaping renders the primary face's `.notdef`.
    Missing,
}

/// One maximal byte range resolved to one face for one reason.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FontAssignment {
    pub range: Range<usize>,
    /// `None` for [`FontChoiceReason::Missing`], and for a leading
    /// [`FontChoiceReason::Ignorable`] cluster when the selection is empty.
    pub font: Option<FontId>,
    pub reason: FontChoiceReason,
    /// The script fallback was keyed on: the cluster's own, or inherited from
    /// the preceding specific-script cluster.
    pub script: Option<ScriptTag>,
    pub emoji: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn language_specific_script_rules_come_before_general_ones() {
        let mut policy = FallbackPolicy::empty();
        policy
            .push_script_rule(HANI, None, ["Han SC"])
            .push_script_rule(HANI, Some("ja"), ["Han JP", "Han SC"])
            .push_script_rule(HANI, Some("zh-Hant"), ["Han TC"]);
        let ja = LanguageTag::new("ja-JP");
        assert_eq!(
            policy.script_families(HANI, ja.as_ref()),
            vec![Arc::from("Han JP"), Arc::from("Han SC")]
        );
        let hk = LanguageTag::new("zh-Hant-HK");
        assert_eq!(
            policy.script_families(HANI, hk.as_ref()),
            vec![Arc::from("Han TC"), Arc::from("Han SC")]
        );
        assert_eq!(
            policy.script_families(HANI, None),
            vec![Arc::from("Han SC")]
        );
        assert!(policy.script_families(HANG, None).is_empty());
    }

    #[test]
    fn system_ui_and_emoji_generics_fall_back_to_their_natural_lists() {
        let mut policy = FallbackPolicy::empty();
        policy
            .set_generic(GenericFamily::SansSerif, ["UI Sans"])
            .set_emoji_families(["Color Emoji"]);
        assert_eq!(
            policy.generic(GenericFamily::SystemUi),
            &[Arc::from("UI Sans")]
        );
        assert_eq!(
            policy.generic(GenericFamily::Emoji),
            &[Arc::from("Color Emoji")]
        );
        assert!(policy.generic(GenericFamily::Serif).is_empty());
    }
}
