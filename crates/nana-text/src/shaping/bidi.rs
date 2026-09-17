//! The Unicode Bidirectional Algorithm, via `unicode-bidi`. The only file
//! allowed to name it.
//!
//! Shaping needs two things from UBA: the resolved embedding level of every
//! byte (rules P–I, per paragraph), and — for Phase 3 line reordering — rule
//! L2 over a sequence of run levels. Nothing here reorders glyphs itself.

use super::ShapedParagraph;
use unicode_bidi::{BidiInfo, Level};

/// The characters UBA ends a paragraph at (bidi class B).
///
/// This is the authority for the whole crate: shaping drops them (they produce
/// no glyph), layout ends a line at each one, and `preserve_lines: false` folds
/// the one-byte ones to a space. A list that disagrees with what
/// `unicode-bidi` actually splits on would leave a separator inside a line,
/// drawn as `.notdef` — `the_separator_list_is_what_unicode_bidi_splits_on`
/// pins it against the real data.
///
/// `\r\n` is this list's only multi-character case, and it is a pair of
/// members rather than a seventh entry.
pub const PARAGRAPH_SEPARATORS: [char; 7] = [
    '\n', '\r', '\u{1c}', '\u{1d}', '\u{1e}', '\u{85}', '\u{2029}',
];

/// Resolved levels for a whole text.
pub struct Levels {
    /// One level per byte of the text.
    pub levels: Vec<u8>,
    pub paragraphs: Vec<ShapedParagraph>,
}

/// Resolves embedding levels with every paragraph's base level fixed by
/// `rtl`, as CSS `direction` does (no first-strong detection).
pub fn resolve(text: &str, rtl: bool) -> Levels {
    let base = if rtl { Level::rtl() } else { Level::ltr() };
    let info = BidiInfo::new(text, Some(base));
    Levels {
        levels: info.levels.iter().map(|level| level.number()).collect(),
        paragraphs: info
            .paragraphs
            .iter()
            .map(|paragraph| ShapedParagraph {
                range: paragraph.range.clone(),
                base_level: paragraph.level.number(),
            })
            .collect(),
    }
}

/// Rule L2: the visual order of items given their embedding levels, as indices
/// into `levels`.
///
/// `levels` must already have rule L1 applied by the caller (trailing
/// whitespace and separators reset to the paragraph level) if the items end a
/// line; Phase 3 owns that.
pub fn visual_order(levels: &[u8]) -> Vec<usize> {
    let levels: Vec<Level> = levels
        .iter()
        .map(|level| Level::new(*level).unwrap_or_else(|_| Level::ltr()))
        .collect();
    BidiInfo::reorder_visual(&levels)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn declared_direction_fixes_the_paragraph_level_and_arabic_nests_inside_latin() {
        let text = "abc عربي def";
        let ltr = resolve(text, false);
        assert_eq!(ltr.paragraphs[0].base_level, 0);
        assert_eq!(ltr.levels[0], 0);
        assert_eq!(
            ltr.levels[4], 1,
            "Arabic is one level above an LTR paragraph"
        );

        let rtl = resolve(text, true);
        assert_eq!(rtl.paragraphs[0].base_level, 1);
        assert_eq!(rtl.levels[0], 2, "Latin nests at 2 inside an RTL paragraph");
        assert_eq!(rtl.levels[4], 1);
    }

    #[test]
    fn newlines_split_paragraphs() {
        let levels = resolve("one\ntwo", false);
        assert_eq!(levels.paragraphs.len(), 2);
        assert_eq!(levels.paragraphs[0].range, 0..4);
        assert_eq!(levels.paragraphs[1].range, 4..7);
    }

    /// Every character `unicode-bidi` starts a new paragraph at is in
    /// [`PARAGRAPH_SEPARATORS`], and every member really does start one.
    #[test]
    fn the_separator_list_is_what_unicode_bidi_splits_on() {
        for candidate in ('\u{0}'..='\u{2100}').chain(['\u{2028}', '\u{2029}']) {
            let text = format!("a{candidate}b");
            let splits = resolve(&text, false).paragraphs.len() > 1;
            assert_eq!(
                splits,
                PARAGRAPH_SEPARATORS.contains(&candidate),
                "U+{:04X} splits paragraphs: {splits}, listed: {}",
                candidate as u32,
                PARAGRAPH_SEPARATORS.contains(&candidate)
            );
        }
    }

    #[test]
    fn rule_l2_reverses_odd_level_runs() {
        assert_eq!(visual_order(&[0, 1, 1, 0]), vec![0, 2, 1, 3]);
        assert_eq!(visual_order(&[1, 2, 1]), vec![2, 1, 0]);
    }
}
