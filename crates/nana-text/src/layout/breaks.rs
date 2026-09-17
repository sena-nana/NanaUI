//! UAX #14 line break opportunities, via `unicode-linebreak`. The only file
//! allowed to name it.
//!
//! Layout asks one question here: at which byte offsets of a paragraph may a
//! line start. Which of those offsets a line actually breaks at is decided on
//! shaped advances, never on this data, and never on a codepoint count.

use unicode_linebreak::linebreaks;

/// Byte offsets, relative to `base`, where a line may start.
///
/// The offsets strictly inside the paragraph only: offset 0 is where the
/// paragraph already starts, and the mandatory opportunity `unicode-linebreak`
/// always reports at the end of the text is the paragraph's own end. Hard
/// breaks come from the paragraph structure the BiDi pass produced, so a
/// paragraph's interior carries no mandatory break of its own.
pub fn opportunities(text: &str, base: usize) -> Vec<usize> {
    linebreaks(text)
        .filter(|(offset, _)| *offset > 0 && *offset < text.len())
        .map(|(offset, _)| base + offset)
        .collect()
}

/// Characters that force a line break **inside** a BiDi paragraph.
///
/// UAX #14's mandatory breaks are LF, CR, NL, VT, FF, LS and PS. The BiDi pass
/// splits paragraphs at LF, CR, NL and PS, so the paragraph structure already
/// carries those; only these three can force a break it does not.
///
/// `forced_breaks_agree_with_uax14` pins this list against the crate's own
/// mandatory-break data, so a Unicode revision that adds one cannot slip past.
pub const FORCED_BREAKS: [char; 3] = ['\u{b}', '\u{c}', '\u{2028}'];

/// True when `text` carries a break the paragraph structure does not.
///
/// A plain character scan, not a UAX #14 pass: the answer decides whether a
/// paragraph needs splitting at all, and the Label fast path asks it once per
/// layout it builds.
pub fn has_forced_break(text: &str) -> bool {
    text.contains(FORCED_BREAKS)
}

/// Mandatory break offsets, as `unicode-linebreak` reports them.
#[cfg(test)]
pub fn mandatory(text: &str) -> Vec<usize> {
    use unicode_linebreak::BreakOpportunity;
    linebreaks(text)
        .filter(|(_, opportunity)| *opportunity == BreakOpportunity::Mandatory)
        .map(|(offset, _)| offset)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_space_offers_a_break_before_the_next_word_and_not_at_either_end() {
        assert_eq!(opportunities("the quick brown", 0), vec![4, 10]);
        assert_eq!(opportunities("the quick brown", 100), vec![104, 110]);
    }

    #[test]
    fn han_breaks_between_every_ideograph_without_any_space() {
        assert_eq!(opportunities("中文排版", 0), vec![3, 6, 9]);
    }

    #[test]
    fn a_word_with_no_interior_opportunity_reports_none() {
        assert!(opportunities("Hamburgefonstiv", 0).is_empty());
    }

    #[test]
    fn a_newline_is_the_only_mandatory_break_inside_the_text() {
        assert_eq!(mandatory("one\ntwo"), vec![4, 7]);
    }

    /// Every character UAX #14 breaks at is either a BiDi paragraph separator
    /// or one of [`FORCED_BREAKS`]. Nothing else may force a break, and none of
    /// these three may stop forcing one.
    #[test]
    fn forced_breaks_agree_with_uax14() {
        // The separators the BiDi pass splits paragraphs at.
        const PARAGRAPH_SEPARATORS: [char; 4] = ['\n', '\r', '\u{85}', '\u{2029}'];
        for candidate in ('\u{0}'..='\u{2100}').chain(['\u{2028}', '\u{2029}']) {
            let text = format!("a{candidate}b");
            let end = text.len();
            let breaks_here = mandatory(&text).iter().any(|offset| *offset < end);
            let expected =
                FORCED_BREAKS.contains(&candidate) || PARAGRAPH_SEPARATORS.contains(&candidate);
            assert_eq!(
                breaks_here, expected,
                "U+{:04X} forces a break: {breaks_here}, listed: {expected}",
                candidate as u32
            );
        }
    }

    #[test]
    fn a_forced_break_is_found_without_a_uax14_pass() {
        assert!(has_forced_break("one\u{2028}two"));
        assert!(has_forced_break("one\u{c}two"));
        assert!(!has_forced_break("one two"));
        assert!(
            !has_forced_break("one\ntwo"),
            "a newline is the paragraph structure's break, not this one"
        );
    }
}
