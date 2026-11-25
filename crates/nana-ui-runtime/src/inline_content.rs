//! Canonical content, independent of authoring history and parent layout.
//! Strings/spans use the existing rich-text representation. Atomic entries
//! reference ordinary retained nodes; they are never wrapper components.

use crate::StableNodeId;
use crate::layout_engine::{LayoutViewport, Size};
use nana_ui_core::sort::CompactSort as _;
use nana_ui_core::{RichSpanStyle, RichText};
use std::{ops::Range, sync::Arc};

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct InlineTextSemantics {
    pub accessible_text: Option<Arc<str>>,
    pub copy_text: Option<Arc<str>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InlineAtomic {
    /// Byte offset of this child's U+FFFC in the physical text source.
    pub offset: usize,
    pub node: StableNodeId,
    pub text: InlineTextSemantics,
}

#[derive(Debug, Clone, PartialEq)]
pub struct MixedInlineContent {
    text: RichText,
    atoms: Arc<[InlineAtomic]>,
}

/// Optimized forms of one content contract. Callers cannot choose a slower
/// retained form: every mutation normalizes before entering UiWorld.
#[derive(Debug, Clone, PartialEq)]
pub enum InlineContent {
    PlainText(Arc<str>),
    StyledText(RichText),
    Mixed(Arc<MixedInlineContent>),
}

impl Default for InlineContent {
    fn default() -> Self {
        Self::PlainText(Arc::from(""))
    }
}
impl From<&str> for InlineContent {
    fn from(text: &str) -> Self {
        Self::PlainText(Arc::from(text))
    }
}
impl From<String> for InlineContent {
    fn from(text: String) -> Self {
        Self::PlainText(text.into())
    }
}
impl From<Arc<str>> for InlineContent {
    fn from(text: Arc<str>) -> Self {
        Self::PlainText(text)
    }
}
impl From<RichText> for InlineContent {
    fn from(text: RichText) -> Self {
        Self::from_parts(text, Arc::from([]))
    }
}

impl InlineContent {
    pub fn builder() -> InlineContentBuilder {
        InlineContentBuilder::default()
    }
    pub fn text(&self) -> &str {
        match self {
            Self::PlainText(text) => text,
            Self::StyledText(text) => text.text(),
            Self::Mixed(content) => content.text.text(),
        }
    }
    pub fn shared_text(&self) -> &Arc<str> {
        match self {
            Self::PlainText(text) => text,
            Self::StyledText(text) => text.shared_text(),
            Self::Mixed(content) => content.text.shared_text(),
        }
    }
    pub fn atoms(&self) -> &[InlineAtomic] {
        match self {
            Self::Mixed(content) => &content.atoms,
            _ => &[],
        }
    }
    pub fn rich_text(&self) -> Option<&RichText> {
        match self {
            Self::StyledText(text) => Some(text),
            Self::Mixed(content) => Some(&content.text),
            _ => None,
        }
    }
    pub fn to_rich_text(&self) -> RichText {
        match self {
            Self::PlainText(text) => RichText::new(Arc::clone(text)),
            Self::StyledText(text) => text.clone(),
            Self::Mixed(content) => content.text.clone(),
        }
    }
    pub fn normalized(self) -> Self {
        match self {
            Self::StyledText(text) => Self::from(text),
            other => other,
        }
    }
    pub(crate) fn from_parts(text: RichText, atoms: Arc<[InlineAtomic]>) -> Self {
        if !atoms.is_empty() {
            Self::Mixed(Arc::new(MixedInlineContent { text, atoms }))
        } else if text.spans().is_empty()
            && text.objects().is_empty()
            && text.rubies().is_empty()
            && text.paragraph_alignments().is_empty()
        {
            Self::PlainText(Arc::clone(text.shared_text()))
        } else {
            Self::StyledText(text)
        }
    }
    /// Replaces a range's style. Empty/default style removes historical spans
    /// and returns plain text automatically when the last span disappears.
    pub fn with_span(&self, range: Range<usize>, style: RichSpanStyle) -> Self {
        let mut text = self.to_rich_text();
        text.set_span(range, style);
        Self::from_parts(text, self.atoms().into())
    }
    /// Replaces source text or atomic entries through the same canonical
    /// operation as direct construction. Removed text is not retained.
    pub fn replace(&self, range: Range<usize>, piece: impl Into<InlineContent>) -> Self {
        let piece = piece.into().normalized();
        let range = snap(self.text(), range);
        let mut text = self.to_rich_text();
        text.replace_with(range.clone(), &piece.to_rich_text());
        let mut atoms: Vec<_> = self
            .atoms()
            .iter()
            .filter(|atom| !range.contains(&atom.offset))
            .cloned()
            .map(|mut atom| {
                if atom.offset >= range.end {
                    atom.offset = atom.offset + piece.text().len() - range.len();
                }
                atom
            })
            .collect();
        atoms.extend(piece.atoms().iter().cloned().map(|mut atom| {
            atom.offset += range.start;
            atom
        }));
        atoms.compact_sort_by_key(|atom| atom.offset);
        Self::from_parts(text, atoms.into())
    }
    pub fn atomic(node: StableNodeId) -> Self {
        Self::atomic_with_text(node, InlineTextSemantics::default())
    }
    pub fn atomic_with_text(node: StableNodeId, text: InlineTextSemantics) -> Self {
        Self::from_parts(
            RichText::new("\u{fffc}"),
            Arc::from([InlineAtomic {
                offset: 0,
                node,
                text,
            }]),
        )
    }
    /// A normalized stream for inspection and source mapping. Text ranges
    /// exclude atomic placeholders; the text engine owns their clusters.
    pub fn items(&self) -> Vec<InlineItem> {
        let mut out = Vec::with_capacity(self.atoms().len() * 2 + 1);
        let mut at = 0;
        for atom in self.atoms() {
            if at < atom.offset {
                out.push(InlineItem::Text(at..atom.offset));
            }
            out.push(InlineItem::Atomic(atom.clone()));
            at = atom.offset + '\u{fffc}'.len_utf8();
        }
        if at < self.text().len() {
            out.push(InlineItem::Text(at..self.text().len()));
        }
        out
    }
}

fn snap(text: &str, range: Range<usize>) -> Range<usize> {
    let boundary = |at: usize| {
        let mut at = at.min(text.len());
        while !text.is_char_boundary(at) {
            at -= 1;
        }
        at
    };
    let start = boundary(range.start);
    start..boundary(range.end).max(start)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InlineItem {
    Text(Range<usize>),
    Atomic(InlineAtomic),
}

/// Authoring scratch only. It is discarded when the canonical value is built;
/// nested pieces and adjacent compatible text never remain as wrapper state.
#[derive(Debug, Default)]
pub struct InlineContentBuilder {
    pieces: Vec<InlineContent>,
}
impl InlineContentBuilder {
    pub fn text(mut self, text: impl Into<Arc<str>>) -> Self {
        self.push(InlineContent::PlainText(text.into()));
        self
    }
    pub fn styled(mut self, text: impl Into<Arc<str>>, style: RichSpanStyle) -> Self {
        let text: Arc<str> = text.into();
        let len = text.len();
        self.push(InlineContent::from(
            RichText::new(text).with_span(0..len, style),
        ));
        self
    }
    pub fn child(mut self, node: StableNodeId) -> Self {
        self.push(InlineContent::atomic(node));
        self
    }
    pub fn child_with_text(mut self, node: StableNodeId, text: InlineTextSemantics) -> Self {
        self.push(InlineContent::atomic_with_text(node, text));
        self
    }
    pub fn line_break(self) -> Self {
        self.text("\n")
    }
    pub fn span(mut self, content: impl Into<InlineContent>) -> Self {
        self.push(content.into());
        self
    }
    fn push(&mut self, content: InlineContent) {
        if !content.text().is_empty() {
            self.pieces.push(content.normalized());
        }
    }
    pub fn build(mut self) -> InlineContent {
        if self.pieces.len() == 1 {
            return self.pieces.pop().unwrap();
        }
        let mut source =
            String::with_capacity(self.pieces.iter().map(|piece| piece.text().len()).sum());
        for piece in &self.pieces {
            source.push_str(piece.text());
        }
        let mut text = RichText::new(source);
        let mut atoms = Vec::new();
        let mut at = 0;
        let mut paragraph = 0;
        for piece in self.pieces {
            if let Some(rich) = piece.rich_text() {
                for (range, style) in rich.spans().iter() {
                    text.set_span(at + range.start..at + range.end, style.clone());
                }
                for (offset, object) in rich.objects() {
                    text.set_object(at + offset, object.clone());
                }
                for (range, ruby) in rich.rubies() {
                    text.set_ruby(at + range.start..at + range.end, Arc::clone(ruby));
                }
                for (index, align) in rich.paragraph_alignments().iter().enumerate() {
                    text.set_paragraph_alignment(paragraph + index, *align);
                }
            }
            atoms.extend(piece.atoms().iter().cloned().map(|mut atom| {
                atom.offset += at;
                atom
            }));
            paragraph += piece.text().bytes().filter(|b| *b == b'\n').count();
            at += piece.text().len();
        }
        InlineContent::from_parts(text, atoms.into())
    }
}

/// Derived text-engine inputs, stored only for mixed paragraphs. The canonical
/// content stays in the existing text/span stores plus the atomic sequence.
#[derive(Debug, Clone, Default)]
pub(crate) struct InlineRecord {
    pub atoms: Arc<[InlineAtomic]>,
    pub objects: Arc<[nana_text::InlineObject]>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct ParentLineStrut {
    pub(crate) font_ascent: f32,
    pub(crate) font_descent: f32,
    pub(crate) x_height: f32,
    pub(crate) font_px: f32,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum AtomLinePlacement {
    Baseline { above: f32, below: f32 },
    Top { height: f32 },
    Bottom { height: f32 },
}

/// Resolves both native-paragraph `nana-text::InlineObjectMetrics` and block IFC
/// `AtomLinePlacement` from one CSS `vertical-align` model.
#[allow(clippy::too_many_arguments)]
pub(crate) fn resolve_atom_vertical_align(
    child_style: &nana_ui_core::LayoutStyle,
    writing: nana_ui_core::WritingContext,
    used: Size,
    margin: nana_ui_core::PaddingSpec,
    own_baseline: Option<f32>,
    parent_strut: ParentLineStrut,
    parent_font_px: f32,
    viewport: LayoutViewport,
) -> (nana_text::InlineObjectMetrics, AtomLinePlacement) {
    let (inline, block) = writing.logical_size(
        used.width + margin.left + margin.right,
        used.height + margin.top + margin.bottom,
    );
    let width_px = inline.max(0.0);
    let block = block.max(0.0);
    if writing.is_vertical() {
        return (
            nana_text::InlineObjectMetrics {
                width_px,
                ascent_px: block,
                descent_px: 0.0,
                align: nana_text::InlineObjectAlign::Baseline,
                baseline_shift_px: 0.0,
                envelope: None,
            },
            AtomLinePlacement::Baseline {
                above: block,
                below: 0.0,
            },
        );
    }
    let spec = child_style.resolved_vertical_align();
    let own_above = if spec.uses_own_baseline() {
        own_baseline.map_or(block, |baseline| {
            (baseline.clamp(0.0, used.height.max(0.0)) + margin.top).clamp(0.0, block)
        })
    } else {
        block
    };
    let own_below = block - own_above;
    match spec {
        nana_ui_core::VerticalAlignSpec::Top => (
            nana_text::InlineObjectMetrics {
                width_px,
                ascent_px: block,
                descent_px: 0.0,
                align: nana_text::InlineObjectAlign::Top,
                baseline_shift_px: 0.0,
                envelope: None,
            },
            AtomLinePlacement::Top { height: block },
        ),
        nana_ui_core::VerticalAlignSpec::Bottom => (
            nana_text::InlineObjectMetrics {
                width_px,
                ascent_px: own_above,
                descent_px: own_below,
                align: nana_text::InlineObjectAlign::Bottom,
                baseline_shift_px: 0.0,
                envelope: None,
            },
            AtomLinePlacement::Bottom { height: block },
        ),
        other => {
            let (above, below, shift) = match other {
                nana_ui_core::VerticalAlignSpec::Baseline => (own_above, own_below, 0.0),
                nana_ui_core::VerticalAlignSpec::Sub => {
                    let shift = -(parent_strut.font_px.max(0.0) * 0.2);
                    (own_above + shift, own_below - shift, shift)
                }
                nana_ui_core::VerticalAlignSpec::Super => {
                    let shift = parent_strut.font_px.max(0.0) / 3.0;
                    (own_above + shift, own_below - shift, shift)
                }
                nana_ui_core::VerticalAlignSpec::TextTop => {
                    let above = parent_strut.font_ascent;
                    let below = block - above;
                    (above, below, above - own_above)
                }
                nana_ui_core::VerticalAlignSpec::TextBottom => {
                    let below = parent_strut.font_descent;
                    let above = block - below;
                    (above, below, above - own_above)
                }
                nana_ui_core::VerticalAlignSpec::Middle => {
                    let half_x = parent_strut.x_height * 0.5;
                    let above = block * 0.5 + half_x;
                    let below = block * 0.5 - half_x;
                    (above, below, above - own_above)
                }
                nana_ui_core::VerticalAlignSpec::Offset(len) => {
                    let child_fonts = nana_ui_core::FontSizeContext::new(
                        nana_ui_core::FontSizeContext::DEFAULT_ROOT_PX,
                        child_style.font_size.unwrap_or(parent_font_px),
                    );
                    let child_line_height = nana_ui_core::text_line_box_height_px(
                        child_fonts.element_px,
                        child_style.line_height,
                    );
                    let shift = len
                        .resolve_with_fonts(
                            Some(child_line_height),
                            Some((viewport.width, viewport.height)),
                            child_fonts,
                        )
                        .unwrap_or(0.0);
                    (own_above + shift, own_below - shift, shift)
                }
                nana_ui_core::VerticalAlignSpec::Top | nana_ui_core::VerticalAlignSpec::Bottom => {
                    unreachable!()
                }
            };
            (
                nana_text::InlineObjectMetrics {
                    width_px,
                    ascent_px: own_above,
                    descent_px: own_below,
                    align: nana_text::InlineObjectAlign::Baseline,
                    baseline_shift_px: shift,
                    envelope: None,
                },
                AtomLinePlacement::Baseline { above, below },
            )
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn child() -> StableNodeId {
        StableNodeId::new(7).unwrap()
    }
    #[test]
    fn construction_edits_and_restored_spans_have_the_same_canonical_value() {
        let style = RichSpanStyle::new().size(22.0);
        let direct = InlineContent::builder()
            .text("A中")
            .styled("tail", style.clone())
            .child(child())
            .build();
        let edited = InlineContent::from("discard")
            .replace(0..7, "A中tail")
            .with_span(4..8, style)
            .replace(8..8, InlineContent::atomic(child()));
        assert_eq!(direct, edited);
        let plain = InlineContent::from("A中tail");
        assert_eq!(
            plain
                .with_span(4..8, RichSpanStyle::new().size(22.0))
                .with_span(4..8, RichSpanStyle::default()),
            plain
        );
        assert_eq!(
            direct
                .replace(8..11, "")
                .with_span(4..8, RichSpanStyle::default()),
            plain
        );
        assert!(matches!(plain, InlineContent::PlainText(_)));
    }
    #[test]
    fn replacement_shifts_utf8_object_source_without_retaining_removed_text() {
        let content = InlineContent::builder()
            .text("old ")
            .child(child())
            .text(" end")
            .build();
        let replaced = content.replace(0..4, "新");
        assert_eq!(replaced.atoms()[0].offset, 3);
        assert_eq!(
            replaced.items(),
            vec![
                InlineItem::Text(0..3),
                InlineItem::Atomic(replaced.atoms()[0].clone()),
                InlineItem::Text(6..10)
            ]
        );
        assert_eq!(replaced.replace(3..6, "").text(), "新 end");
    }
}
