//! Where two versions of a text differ: the one prefix/suffix scan every
//! consumer that is handed a whole new value uses to find the edit in it.

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
