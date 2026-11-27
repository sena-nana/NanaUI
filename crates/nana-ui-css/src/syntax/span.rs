//! Byte-offset source spans and line/column resolution.

/// Half-open byte range `[start, end)` into the original UTF-8 CSS source `&str`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct Span {
    pub start: usize,
    pub end: usize,
}

impl Span {
    /// Empty zero-length span at byte 0.
    pub const EMPTY: Self = Self { start: 0, end: 0 };

    /// Construct a span from `[start, end)`.
    #[inline]
    pub const fn new(start: usize, end: usize) -> Self {
        Self { start, end }
    }

    /// Length in bytes.
    #[inline]
    pub const fn len(self) -> usize {
        self.end.saturating_sub(self.start)
    }

    /// Returns `true` when the span covers zero bytes.
    #[inline]
    pub const fn is_empty(self) -> bool {
        self.start >= self.end
    }

    /// Return the smallest span covering both `self` and `other`.
    #[inline]
    pub const fn merge(self, other: Self) -> Self {
        let start = if self.start < other.start {
            self.start
        } else {
            other.start
        };
        let end = if self.end > other.end {
            self.end
        } else {
            other.end
        };
        Self { start, end }
    }

    /// Slice `src` safely, returning `""` if out of bounds or not on UTF-8 char boundaries.
    #[inline]
    pub fn slice(self, src: &str) -> &str {
        if self.start <= self.end
            && self.end <= src.len()
            && src.is_char_boundary(self.start)
            && src.is_char_boundary(self.end)
        {
            &src[self.start..self.end]
        } else {
            ""
        }
    }

    /// Compute 1-based `(line, col)` at `self.start` in `src`.
    ///
    /// Line breaks follow CSS Syntax Level 3 preprocessing: `\r\n`, `\n`, `\r`,
    /// and `\u{000C}` (form feed) each advance the line number by 1. Columns
    /// count Unicode scalar values (`char`s), starting at 1.
    pub fn line_col(&self, src: &str) -> (usize, usize) {
        let limit = self.start.min(src.len());
        let mut line = 1usize;
        let mut col = 1usize;
        let mut chars = src.char_indices().peekable();
        while let Some((idx, ch)) = chars.next() {
            if idx >= limit {
                break;
            }
            match ch {
                '\r' => {
                    if matches!(chars.peek(), Some((next_idx, '\n')) if *next_idx < limit) {
                        chars.next();
                    }
                    line += 1;
                    col = 1;
                }
                '\n' | '\u{000C}' => {
                    line += 1;
                    col = 1;
                }
                _ => {
                    col += 1;
                }
            }
        }
        (line, col)
    }
}
