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

/// Whether a break at `offset` would be mandatory. Only used by the tests that
/// pin the tailoring; layout itself takes hard breaks from the paragraph
/// structure.
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
}
