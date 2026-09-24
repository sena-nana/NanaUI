//! Where two versions of a text differ: the one prefix/suffix scan every
//! consumer that is handed a whole new value uses to find the edit in it.

use std::ops::Range;

/// The byte range `previous` and `next` differ in, as
/// `(start, previous_end, next_end)`: the bytes before `start`, and the bytes
/// after the two ends, are identical. `None` when the two are the same text.
///
/// All three are character boundaries in their own string, so callers can
/// slice with them. A change inside one character therefore reports the whole
/// character: replacing 好 (`E5 A5 BD`) with 奿 (`E5 A5 BF`) shares two bytes,
/// but the range covers all three.
pub fn changed_range(previous: &str, next: &str) -> Option<(usize, usize, usize)> {
    // The common case is "nothing changed", and `==` answers it a whole
    // vector register at a time where the byte-wise scans below cannot: they
    // only stop early when there IS a difference.
    if previous == next {
        return None;
    }
    let (old, new) = (previous.as_bytes(), next.as_bytes());
    let mut start = common_prefix(old, new);
    let mut suffix = common_suffix(&old[start..], &new[start..]);
    // Snap onto character boundaries. The bytes below `start` are the same in
    // both strings, so a boundary there is a boundary in both; the bytes from
    // the two ends on are the same too, so one `suffix` answers for both ends
    // (a boundary is decided by the byte at it, and those bytes are equal).
    while start > 0 && !previous.is_char_boundary(start) {
        start -= 1;
    }
    while suffix > 0 && !previous.is_char_boundary(old.len() - suffix) {
        suffix -= 1;
    }
    Some((start, old.len() - suffix, new.len() - suffix))
}

/// [`changed_range`], placed clear of `keep` when the change can be.
///
/// A pure insertion or deletion next to text it repeats has several honest
/// placements: `"aa"` → `"a"` lost either `a`. The greedy scan puts it as far
/// right as the common prefix reaches, which can land it on a composition
/// that an equally valid placement leaves alone. When the greedy placement
/// reaches `keep` — overlaps it, or inserts strictly inside it (the rule a
/// composition survives by) — this slides the change to just before or just
/// after `keep` instead, if the repeated text allows it.
pub fn changed_range_clear_of(
    previous: &str,
    next: &str,
    keep: Range<usize>,
) -> Option<(usize, usize, usize)> {
    let greedy = changed_range(previous, next)?;
    let reaches = |(start, old_end, _): (usize, usize, usize)| {
        (start < keep.end && old_end > keep.start)
            || (start == old_end && start > keep.start && start < keep.end)
    };
    if !reaches(greedy) {
        return Some(greedy);
    }
    let (old, new) = (previous.as_bytes(), next.as_bytes());
    let shorter = old.len().min(new.len());
    let (prefix, suffix) = (common_prefix(old, new), common_suffix(old, new));
    if prefix + suffix < shorter {
        // Something was replaced, not only added or removed: one placement.
        return Some(greedy);
    }
    let (removed, inserted) = (old.len() - shorter, new.len() - shorter);
    // Any start in `shorter - suffix ..= prefix` explains the change.
    let lowest = shorter - suffix;
    [keep.start.saturating_sub(removed), keep.end]
        .into_iter()
        .filter(|&start| start >= lowest && start <= prefix)
        .map(|start| (start, start + removed, start + inserted))
        .find(|&(start, old_end, new_end)| {
            previous.is_char_boundary(start)
                && previous.is_char_boundary(old_end)
                && next.is_char_boundary(start)
                && next.is_char_boundary(new_end)
                && !reaches((start, old_end, new_end))
        })
        .or(Some(greedy))
}

/// Bytes at the front of both slices that are equal.
///
/// Block compared rather than zipped byte by byte: `==` on a slice is a
/// vectorised memcmp, an iterator of byte pairs is not, and an edit far from
/// the front makes the scan walk the whole document to find out.
fn common_prefix(old: &[u8], new: &[u8]) -> usize {
    const BLOCK: usize = 64;
    let max = old.len().min(new.len());
    let mut matched = 0;
    while matched + BLOCK <= max && old[matched..matched + BLOCK] == new[matched..matched + BLOCK] {
        matched += BLOCK;
    }
    while matched < max && old[matched] == new[matched] {
        matched += 1;
    }
    matched
}

/// Bytes at the end of both slices that are equal. See [`common_prefix`].
fn common_suffix(old: &[u8], new: &[u8]) -> usize {
    const BLOCK: usize = 64;
    let max = old.len().min(new.len());
    let mut matched = 0;
    let block = |slice: &[u8], from_end: usize| from_end + BLOCK <= slice.len();
    while matched + BLOCK <= max
        && block(old, matched)
        && block(new, matched)
        && old[old.len() - matched - BLOCK..old.len() - matched]
            == new[new.len() - matched - BLOCK..new.len() - matched]
    {
        matched += BLOCK;
    }
    while matched < max && old[old.len() - matched - 1] == new[new.len() - matched - 1] {
        matched += 1;
    }
    matched
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn equal_texts_have_no_changed_range() {
        assert_eq!(changed_range("abc", "abc"), None);
    }

    #[test]
    fn a_repeated_change_slides_clear_of_what_it_must_keep() {
        // Deleting one of "ab ab": greedily the second, which a composition
        // over 2..3 would lose to; the first is just as true.
        assert_eq!(changed_range("abab", "ab"), Some((2, 4, 2)));
        assert_eq!(changed_range_clear_of("abab", "ab", 2..3), Some((0, 2, 0)));
        // Greedy placements already clear of the span are kept.
        assert_eq!(changed_range("aaa", "aaaa"), Some((3, 3, 4)));
        assert_eq!(changed_range_clear_of("aaa", "aaaa", 3..3), Some((3, 3, 4)));
        assert_eq!(changed_range_clear_of("aaa", "aaaa", 1..3), Some((3, 3, 4)));
        assert_eq!(changed_range("aab", "aaab"), Some((2, 2, 3)));
        assert_eq!(changed_range_clear_of("aab", "aaab", 1..2), Some((2, 2, 3)));
        // An insertion strictly inside the span slides to its start.
        assert_eq!(changed_range_clear_of("aab", "aaab", 1..3), Some((1, 1, 2)));
        // A replacement has one placement: kept even if it reaches.
        assert_eq!(changed_range_clear_of("abc", "aXc", 1..2), Some((1, 2, 2)));
        // Only character boundaries: "好好" → "好" slides by whole characters.
        assert_eq!(changed_range_clear_of("好好", "好", 3..4), Some((0, 3, 0)));
    }

    #[test]
    fn the_range_covers_whole_characters() {
        assert_eq!(changed_range("好", "奿"), Some((0, 3, 3)));
        assert_eq!(changed_range("ab", "aXb"), Some((1, 1, 2)));
        let long = "x".repeat(300);
        let edited = format!("{long}!{long}");
        assert_eq!(
            changed_range(&format!("{long}{long}"), &edited),
            Some((300, 300, 301))
        );
    }
}
