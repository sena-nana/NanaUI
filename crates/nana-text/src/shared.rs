//! Text that many holders share without copying, and that says whether two
//! copies are the same bytes without comparing them (Issue #182).
//!
//! An editor's text is read by far more parties than write it: the host's
//! retained geometry, accessibility, the drawable scene, change events, undo.
//! Each used to hold a `String` copy, and each copy was compared byte for byte
//! to learn that nothing changed. [`SharedText`] is one buffer behind an
//! [`Arc`] plus a [`TextStamp`]: a clone is a reference count, and two values
//! with the same stamp are the same bytes, so "is this still the text I laid
//! out" is an integer comparison.

use std::borrow::Borrow;
use std::num::NonZeroU64;
use std::ops::{Deref, Range};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

/// Names one exact byte content, process-wide.
///
/// Drawn from a global counter whenever bytes change and never reused, so
/// equal stamps mean equal bytes. A clone keeps its stamp; two clones that
/// then change each draw a fresh one, which is what keeps them from ever
/// colliding.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct TextStamp(NonZeroU64);

impl TextStamp {
    /// A stamp no other bytes have carried.
    pub fn fresh() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(1);
        let value = NEXT.fetch_add(1, Ordering::Relaxed);
        Self(NonZeroU64::new(value).expect("the stamp counter does not wrap"))
    }

    pub const fn get(self) -> u64 {
        self.0.get()
    }
}

/// An immutable, cheaply cloned string, optionally stamped.
///
/// Values built from a `String` or `&str` are anonymous (no stamp): they never
/// match a cached identity, so a cache keyed on stamps stays correct for them
/// and only loses its fast path. Values that come out of
/// [`EditableText`](crate::EditableText) are stamped.
#[derive(Clone)]
pub struct SharedText {
    buffer: Buffer,
    stamp: Option<TextStamp>,
}

/// Editor text is an `Arc<String>` so an unshared buffer can be edited in
/// place; a label that already lives in an `Arc<str>` is wrapped as it is,
/// without a copy.
#[derive(Clone)]
enum Buffer {
    Owned(Arc<String>),
    Str(Arc<str>),
}

impl Buffer {
    fn as_str(&self) -> &str {
        match self {
            Self::Owned(text) => text,
            Self::Str(text) => text,
        }
    }
}

impl SharedText {
    /// Stamps `text` as a content of its own.
    pub fn stamped(text: impl Into<String>) -> Self {
        Self {
            buffer: Buffer::Owned(Arc::new(text.into())),
            stamp: Some(TextStamp::fresh()),
        }
    }

    pub fn as_str(&self) -> &str {
        self.buffer.as_str()
    }

    /// The stamp naming these bytes; `None` for an anonymous value.
    pub fn stamp(&self) -> Option<TextStamp> {
        self.stamp
    }

    /// Whether both share one buffer.
    pub fn ptr_eq(&self, other: &Self) -> bool {
        let (left, right) = (self.as_str(), other.as_str());
        left.as_ptr() == right.as_ptr() && left.len() == right.len()
    }

    /// Whether both are known to be the same bytes without looking at them.
    pub fn same_identity(&self, other: &Self) -> bool {
        self.ptr_eq(other) || (self.stamp.is_some() && self.stamp == other.stamp)
    }

    /// This value with a stamp, drawing one when it has none.
    #[must_use]
    pub fn into_stamped(mut self) -> Self {
        if self.stamp.is_none() {
            self.stamp = Some(TextStamp::fresh());
        }
        self
    }

    /// The owned `String`, copied only when the buffer is shared.
    pub fn into_string(self) -> String {
        match self.buffer {
            Buffer::Owned(text) => Arc::try_unwrap(text).unwrap_or_else(|shared| (*shared).clone()),
            Buffer::Str(text) => text.to_string(),
        }
    }

    /// Replaces `range` with `text` and draws a fresh stamp. Edits the buffer
    /// in place when nothing else holds it; otherwise builds the new text in
    /// one pass. The caller has checked the range.
    pub(crate) fn replace_range(&mut self, range: Range<usize>, text: &str) {
        let unique = match &mut self.buffer {
            Buffer::Owned(buffer) => Arc::get_mut(buffer),
            Buffer::Str(_) => None,
        };
        match unique {
            Some(buffer) => buffer.replace_range(range, text),
            None => {
                let old = self.buffer.as_str();
                let mut next = String::with_capacity(old.len() - range.len() + text.len());
                next.push_str(&old[..range.start]);
                next.push_str(text);
                next.push_str(&old[range.end..]);
                self.buffer = Buffer::Owned(Arc::new(next));
            }
        }
        self.stamp = Some(TextStamp::fresh());
    }

    /// Applies sorted, disjoint `edits` in one pass and draws a fresh stamp.
    /// The caller has checked every range.
    pub(crate) fn splice(&mut self, edits: &[(Range<usize>, &str)]) {
        let old = self.buffer.as_str();
        let grown: isize = edits
            .iter()
            .map(|(range, text)| text.len() as isize - range.len() as isize)
            .sum();
        let mut next = String::with_capacity((old.len() as isize + grown).max(0) as usize);
        let mut cursor = 0;
        for (range, text) in edits {
            next.push_str(&old[cursor..range.start]);
            next.push_str(text);
            cursor = range.end;
        }
        next.push_str(&old[cursor..]);
        self.buffer = Buffer::Owned(Arc::new(next));
        self.stamp = Some(TextStamp::fresh());
    }
}

fn empty_buffer() -> Buffer {
    static EMPTY: std::sync::OnceLock<Arc<str>> = std::sync::OnceLock::new();
    Buffer::Str(Arc::clone(EMPTY.get_or_init(|| Arc::from(""))))
}

impl Default for SharedText {
    fn default() -> Self {
        Self {
            buffer: empty_buffer(),
            stamp: None,
        }
    }
}

impl Deref for SharedText {
    type Target = str;

    fn deref(&self) -> &str {
        self.buffer.as_str()
    }
}

impl AsRef<str> for SharedText {
    fn as_ref(&self) -> &str {
        self.buffer.as_str()
    }
}

impl Borrow<str> for SharedText {
    fn borrow(&self) -> &str {
        self.buffer.as_str()
    }
}

impl std::fmt::Debug for SharedText {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        std::fmt::Debug::fmt(self.as_str(), f)
    }
}

impl std::fmt::Display for SharedText {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl PartialEq for SharedText {
    fn eq(&self, other: &Self) -> bool {
        self.same_identity(other) || self.as_str() == other.as_str()
    }
}

impl Eq for SharedText {}

impl PartialOrd for SharedText {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for SharedText {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.as_str().cmp(other.as_str())
    }
}

impl std::hash::Hash for SharedText {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.as_str().hash(state);
    }
}

impl PartialEq<str> for SharedText {
    fn eq(&self, other: &str) -> bool {
        self.as_str() == other
    }
}

impl PartialEq<&str> for SharedText {
    fn eq(&self, other: &&str) -> bool {
        self.as_str() == *other
    }
}

impl PartialEq<String> for SharedText {
    fn eq(&self, other: &String) -> bool {
        self.as_str() == other.as_str()
    }
}

impl PartialEq<SharedText> for str {
    fn eq(&self, other: &SharedText) -> bool {
        self == other.as_str()
    }
}

impl PartialEq<SharedText> for &str {
    fn eq(&self, other: &SharedText) -> bool {
        *self == other.as_str()
    }
}

impl PartialEq<SharedText> for String {
    fn eq(&self, other: &SharedText) -> bool {
        self.as_str() == other.as_str()
    }
}

impl From<String> for SharedText {
    fn from(text: String) -> Self {
        if text.is_empty() {
            return Self::default();
        }
        Self {
            buffer: Buffer::Owned(Arc::new(text)),
            stamp: None,
        }
    }
}

impl From<&str> for SharedText {
    fn from(text: &str) -> Self {
        Self::from(text.to_owned())
    }
}

impl From<&String> for SharedText {
    fn from(text: &String) -> Self {
        Self::from(text.as_str())
    }
}

impl From<std::borrow::Cow<'_, str>> for SharedText {
    fn from(text: std::borrow::Cow<'_, str>) -> Self {
        Self::from(text.into_owned())
    }
}

/// Wraps the `Arc<str>` itself: no copy.
impl From<Arc<str>> for SharedText {
    fn from(text: Arc<str>) -> Self {
        Self {
            buffer: Buffer::Str(text),
            stamp: None,
        }
    }
}

impl From<&Arc<str>> for SharedText {
    fn from(text: &Arc<str>) -> Self {
        Self::from(Arc::clone(text))
    }
}

impl From<SharedText> for String {
    fn from(text: SharedText) -> Self {
        text.into_string()
    }
}

/// No copy when the text already lives in an `Arc<str>`.
impl From<&SharedText> for Arc<str> {
    fn from(text: &SharedText) -> Self {
        match &text.buffer {
            Buffer::Str(text) => Arc::clone(text),
            Buffer::Owned(text) => Arc::from(text.as_str()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_clone_shares_the_buffer_and_the_stamp() {
        let text = SharedText::stamped("hello");
        let copy = text.clone();
        assert!(text.ptr_eq(&copy));
        assert_eq!(text.stamp(), copy.stamp());
        assert!(text.same_identity(&copy));
    }

    #[test]
    fn an_edit_of_a_shared_buffer_leaves_the_other_holder_alone() {
        let mut text = SharedText::stamped("hello");
        let snapshot = text.clone();
        text.replace_range(0..1, "J");
        assert_eq!(text, "Jello");
        assert_eq!(snapshot, "hello");
        assert!(!text.ptr_eq(&snapshot));
        assert_ne!(text.stamp(), snapshot.stamp());
    }

    #[test]
    fn an_edit_of_an_unshared_buffer_happens_in_place() {
        let mut text = SharedText::stamped(String::with_capacity(64) + "hello");
        let before = text.as_str().as_ptr();
        let stamp = text.stamp();
        text.replace_range(5..5, " world");
        assert_eq!(text, "hello world");
        assert_eq!(text.as_str().as_ptr(), before, "no reallocation");
        assert_ne!(text.stamp(), stamp, "new bytes, new stamp");
    }

    #[test]
    fn diverging_clones_never_share_a_stamp() {
        let base = SharedText::stamped("abc");
        let mut left = base.clone();
        let mut right = base.clone();
        left.replace_range(0..1, "x");
        right.replace_range(0..1, "x");
        assert_eq!(left, right, "same bytes");
        assert_ne!(left.stamp(), right.stamp(), "but never the same stamp");
    }

    #[test]
    fn anonymous_values_compare_by_content() {
        let anonymous = SharedText::from("abc");
        assert_eq!(anonymous.stamp(), None);
        assert_eq!(anonymous, SharedText::stamped("abc"));
        assert!(!anonymous.same_identity(&SharedText::from("abc")));
        assert_eq!(anonymous, "abc");
        assert_eq!("abc", anonymous);
        assert_eq!(anonymous.into_string(), "abc");
    }

    #[test]
    fn an_arc_str_is_wrapped_without_a_copy() {
        let label: Arc<str> = Arc::from("label");
        let text = SharedText::from(&label);
        assert_eq!(text.as_str().as_ptr(), label.as_ptr());
        let back: Arc<str> = (&text).into();
        assert!(Arc::ptr_eq(&back, &label));
        let mut edited = text.clone().into_stamped();
        edited.replace_range(0..1, "L");
        assert_eq!(edited, "Label");
        assert_eq!(&*label, "label");
    }

    #[test]
    fn a_splice_applies_every_edit_in_one_pass() {
        let mut text = SharedText::stamped("one two three");
        text.splice(&[(0..3, "1"), (4..7, ""), (8..13, "3!")]);
        assert_eq!(text, "1  3!");
    }
}
