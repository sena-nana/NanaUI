//! Grapheme cluster byte-offset index for per-glyph presentation ordinals.

/// Grapheme ordinals of a paragraph's bytes: what per-glyph presentation
/// counts in, so the reveal schedule an application compiled per grapheme
/// lines up with the glyphs whatever the shaper merged or split.
pub(super) struct GraphemeOrdinals {
    starts: Vec<usize>,
}

impl GraphemeOrdinals {
    pub(super) fn new(content: &str) -> Self {
        use unicode_segmentation::UnicodeSegmentation;
        Self {
            starts: content.grapheme_indices(true).map(|(at, _)| at).collect(),
        }
    }

    /// The grapheme `byte` falls in.
    pub(super) fn of(&self, byte: usize) -> u32 {
        self.starts
            .partition_point(|start| *start <= byte)
            .saturating_sub(1) as u32
    }
}
