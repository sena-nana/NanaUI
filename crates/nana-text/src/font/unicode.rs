//! Unicode properties fallback needs, backed by compact static UCD range
//! tables generated from the Unicode Character Database (`font/ucd_tables.rs`),
//! with grapheme clusters from `unicode-segmentation`.

use super::ucd_tables;
use crate::shape::ScriptTag;
use std::cmp::Ordering;
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
    /// Stands upright in vertical text, by UAX #50 on the cluster's base
    /// character. See [`is_upright_in_vertical`].
    pub upright: bool,
}

#[inline]
fn in_bool_ranges(table: &[u64], ch: char) -> bool {
    const END_MASK: u64 = (1 << 21) - 1;
    let cp = u64::from(ch as u32);
    table
        .binary_search_by(|&entry| {
            let start = entry >> 21;
            let end = entry & END_MASK;
            if cp < start {
                Ordering::Greater
            } else if cp > end {
                Ordering::Less
            } else {
                Ordering::Equal
            }
        })
        .is_ok()
}

/// True for codepoints a face need not map to render a cluster: ZWJ, variation
/// selectors, bidi controls and the like.
pub fn is_default_ignorable(ch: char) -> bool {
    in_bool_ranges(ucd_tables::DEFAULT_IGNORABLE, ch)
}

/// True when `ch` is set upright in a vertical line (`text-orientation:
/// mixed`), false when it is set sideways.
///
/// UAX #50 `U` and `Tu` are upright; `R` is sideways. `Tr` — the brackets,
/// the prolonged sound mark, the dashes — is upright too: those need a
/// vertical *form*, which the `vert` feature (or HarfRust's Unicode vertical
/// presentation-form fallback) substitutes on an upright run, and which a
/// rotated run would never ask for.
pub fn is_upright_in_vertical(ch: char) -> bool {
    in_bool_ranges(ucd_tables::VERTICAL_ROTATED, ch) ^ ucd_tables::VERTICAL_INVERTED
}

/// Whether a grapheme cluster takes an emphasis mark (CSS Text Decoration 4
/// `text-emphasis-skip: spaces punctuation`): not when its base character is
/// a separator (`Z*`), punctuation (`P*`), a control or format character
/// (`Cc`, `Cf`), unassigned (`Cn`), or an inline object's U+FFFC.
pub fn takes_emphasis(grapheme: &str) -> bool {
    let Some(base) = grapheme.chars().next() else {
        return false;
    };
    in_bool_ranges(ucd_tables::EMPHASIS_ALLOWED, base)
}

fn specific_script(ch: char) -> Option<ScriptTag> {
    let cp = u64::from(ch as u32);
    let idx = ucd_tables::SCRIPT_RANGES
        .binary_search_by(|&entry| {
            let start = entry >> 32;
            let end = (entry >> 8) & 0xFF_FFFF;
            if cp < start {
                Ordering::Greater
            } else if cp > end {
                Ordering::Less
            } else {
                Ordering::Equal
            }
        })
        .ok()?;
    let tag_idx = (ucd_tables::SCRIPT_RANGES[idx] & 0xFF) as usize;
    Some(ScriptTag(ucd_tables::SCRIPT_TAGS[tag_idx]))
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
    text.grapheme_indices(true)
        .map(|(start, grapheme)| {
            let script = grapheme.chars().find_map(specific_script);
            let has_vs15 = grapheme.contains(VS15_TEXT);
            let emoji = grapheme.contains(VS16_EMOJI)
                || (!has_vs15
                    && grapheme
                        .chars()
                        .next()
                        .is_some_and(|base| in_bool_ranges(ucd_tables::EMOJI_PRESENTATION, base)));
            ClusterInfo {
                range: start..start + grapheme.len(),
                script,
                emoji,
                upright: grapheme.chars().next().is_some_and(is_upright_in_vertical),
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
    fn vertical_orientation_follows_uax_50() {
        // U: Han, kana, emoji. Tu: ideographic full stop. Tr: corner bracket
        // and the prolonged sound mark.
        for upright in ['中', 'あ', '😀', '。', '「', 'ー'] {
            assert!(is_upright_in_vertical(upright), "{upright} is upright");
        }
        // R: Latin, digits, Arabic, the ASCII space.
        for sideways in ['a', '1', 'ب', ' '] {
            assert!(!is_upright_in_vertical(sideways), "{sideways} is sideways");
        }
        let clusters = clusters("中a");
        assert_eq!(
            clusters
                .iter()
                .map(|cluster| cluster.upright)
                .collect::<Vec<_>>(),
            [true, false]
        );
    }

    #[test]
    fn zwj_and_variation_selectors_are_default_ignorable() {
        assert!(is_default_ignorable('\u{200D}'));
        assert!(is_default_ignorable(VS16_EMOJI));
        assert!(!is_default_ignorable('A'));
    }

    #[test]
    fn unicode_versions_align_with_ucd_tables() {
        assert_eq!(ucd_tables::UNICODE_VERSION, (17, 0, 0));
        // `char::UNICODE_VERSION` follows the toolchain, and there is no
        // `rust-toolchain` pin: a newer stable Rust may move to a newer Unicode
        // release. Code points assigned after the table version only gain
        // std-side properties (`char::is_alphanumeric` etc.) that our tables
        // treat as unassigned, which is the same answer older UCD data gives,
        // so a newer std is compatible. A toolchain *older* than the tables
        // would disagree on code points our tables already know, so that stays
        // a failure. Regenerate with `python3 scripts/generate-ucd-tables.py`
        // when moving the table baseline.
        assert!(
            char::UNICODE_VERSION >= ucd_tables::UNICODE_VERSION,
            "toolchain Unicode {:?} is older than ucd_tables {:?}",
            char::UNICODE_VERSION,
            ucd_tables::UNICODE_VERSION
        );
        assert_eq!(unicode_segmentation::UNICODE_VERSION, (17, 0, 0));
        // `harfrust` 0.12.0 exposes no Unicode version constant; its built-in
        // shaping tables are Unicode 17.0.0 (recorded in
        // docs/reference/third-party.md and checked when bumping harfrust).
        // `unicode_bidi::UNICODE_VERSION` (16.0.0) and `unicode_linebreak::UNICODE_VERSION`
        // (15.0.0) are confined by `scripts/check-engine-boundary.py` to
        // `shaping/bidi.rs` and `layout/breaks.rs` respectively.
    }
}
