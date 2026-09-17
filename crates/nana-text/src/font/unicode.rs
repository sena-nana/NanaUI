//! Unicode properties fallback needs, via ICU4X. The only file allowed to name
//! `icu_properties`.
//!
//! Nothing here is a hand-written table: script, `Emoji_Presentation` and
//! `Default_Ignorable_Code_Point` come from ICU's compiled data, grapheme
//! clusters from `unicode-segmentation`.

use crate::shape::ScriptTag;
use icu_properties::props::{DefaultIgnorableCodePoint, EmojiPresentation, Script};
use icu_properties::{CodePointMapData, CodePointSetData, PropertyNamesShort};
use std::ops::Range;
use unicode_segmentation::UnicodeSegmentation;

const VS15_TEXT: char = '\u{FE0E}';
const VS16_EMOJI: char = '\u{FE0F}';

/// One grapheme cluster, classified for fallback.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClusterInfo {
    pub range: Range<usize>,
    /// The first specific (non-Common, non-Inherited) script in the cluster,
    /// or `None` when every codepoint is Common/Inherited.
    pub script: Option<ScriptTag>,
    /// Asks for emoji presentation: VS16, or an `Emoji_Presentation` base not
    /// forced to text by VS15.
    pub emoji: bool,
}

/// True for codepoints a face need not map to render a cluster: ZWJ, variation
/// selectors, bidi controls and the like.
pub fn is_default_ignorable(ch: char) -> bool {
    CodePointSetData::new::<DefaultIgnorableCodePoint>().contains(ch)
}

fn specific_script(ch: char) -> Option<ScriptTag> {
    let script = CodePointMapData::<Script>::new().get(ch);
    if matches!(script, Script::Common | Script::Inherited | Script::Unknown) {
        return None;
    }
    let short = PropertyNamesShort::<Script>::new().get_locale_script(script)?;
    let bytes: [u8; 4] = short.as_str().as_bytes().try_into().ok()?;
    Some(ScriptTag(bytes))
}

/// Byte offsets grapheme clusters start at.
///
/// The boundary data alone, without the script and emoji classification
/// [`clusters`] pays for: layout needs it only to snap a span boundary onto the
/// cluster the shaper would have snapped it to.
pub fn cluster_starts(text: &str) -> Vec<usize> {
    text.grapheme_indices(true)
        .map(|(start, _)| start)
        .collect()
}

/// Grapheme clusters of `text` in logical order.
pub fn clusters(text: &str) -> Vec<ClusterInfo> {
    let emoji_presentation = CodePointSetData::new::<EmojiPresentation>();
    text.grapheme_indices(true)
        .map(|(start, grapheme)| {
            let script = grapheme.chars().find_map(specific_script);
            let has_vs15 = grapheme.contains(VS15_TEXT);
            let emoji = grapheme.contains(VS16_EMOJI)
                || (!has_vs15
                    && grapheme
                        .chars()
                        .next()
                        .is_some_and(|base| emoji_presentation.contains(base)));
            ClusterInfo {
                range: start..start + grapheme.len(),
                script,
                emoji,
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clusters_carry_script_and_emoji_presentation() {
        let text = "a中 👩\u{200D}💻❤\u{FE0F}❤😀\u{FE0E}";
        let clusters = clusters(text);
        let scripts: Vec<Option<&str>> = clusters
            .iter()
            .map(|cluster| cluster.script.as_ref().map(ScriptTag::as_str))
            .collect();
        assert_eq!(
            scripts,
            [Some("Latn"), Some("Hani"), None, None, None, None, None]
        );
        let emoji: Vec<bool> = clusters.iter().map(|cluster| cluster.emoji).collect();
        // ZWJ sequence and VS16 heart are emoji; the bare heart is text by
        // default; VS15 forces the grinning face to text.
        assert_eq!(emoji, [false, false, false, true, true, false, false]);
    }

    #[test]
    fn zwj_and_variation_selectors_are_default_ignorable() {
        assert!(is_default_ignorable('\u{200D}'));
        assert!(is_default_ignorable(VS16_EMOJI));
        assert!(!is_default_ignorable('A'));
    }
}
