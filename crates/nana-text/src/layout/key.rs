//! What a layout depends on, and nothing else.
//!
//! In: the identity of the shaped runs, the container constraints, the text
//! kind, the resolved line box heights and the strut.
//!
//! Out, deliberately: colour, opacity, transform, z-index and background — a
//! paint change must not relayout, and this is the mechanical reason it cannot
//! (none of them appear below, and [`TextStyle`](crate::TextStyle) does not
//! carry them in the first place). Also out: widget identity — 10k labels
//! reading the same string at the same revision share one layout.
//!
//! [`TextRevision`](crate::TextRevision) **is** in, unlike in the shape key.
//! A layout carries the revision it was produced under and
//! [`is_stale`](crate::TextLayout::is_stale) compares it, so a layout handed to
//! a second source must not still claim the first one's revision — that source
//! would read its own current layout as stale on every frame, forever. Two
//! sources holding the same text at different revisions therefore get one
//! layout each; the shaping underneath is still shared, which is where the work
//! is.
//!
//! The shaped text is held by `Arc` and compared by pointer. Holding it is what
//! makes the pointer safe to compare: a freed allocation could otherwise be
//! reused at the same address and alias a different text.

use super::lines::LineStrut;
use crate::constraints::TextConstraints;
use crate::font::canonical_f32_bits;
use crate::id::TextRevision;
use crate::shaping::ShapedText;
use crate::style::TextKind;
use nana_ui_core::{
    DirSpec, LineBreakSpec, TextAlignSpec, TextAutospaceSpec, TextJustifySpec, TextOrientationSpec,
    TextSpacingTrimSpec, TextWrapBreak, TextWrapStyleSpec, WordBreakSpec, WritingModeSpec,
};
use std::hash::{Hash, Hasher};
use std::sync::Arc;

/// The constraint fields, as plain comparable bits.
///
/// Spelled out rather than derived on [`TextConstraints`] so that adding a
/// field to the constraints is a compile error here instead of a cache that
/// silently ignores it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
struct ConstraintsKey {
    max_width: Option<u32>,
    max_height: Option<u32>,
    wrap: u8,
    word_break: u8,
    line_break: u8,
    max_lines: Option<u16>,
    ellipsis: bool,
    preserve_lines: bool,
    base_direction: u8,
    align: u8,
    writing_mode: u8,
    text_orientation: u8,
    /// Keyed although no line applies tab stops yet: the field is a declared
    /// constraint, and a key that ignores one is a cache that hands back the
    /// wrong layout the day it starts mattering.
    tab_width: u8,
    scale: u32,
    spacing_trim: u8,
    autospace: u8,
    justify: u8,
    wrap_style: u8,
}

impl ConstraintsKey {
    fn new(constraints: &TextConstraints) -> Self {
        let TextConstraints {
            max_width_px,
            max_height_px,
            wrap,
            word_break,
            line_break,
            max_lines,
            ellipsis,
            preserve_lines,
            base_direction,
            align,
            writing_mode,
            text_orientation,
            tab_width,
            scale,
            spacing_trim,
            autospace,
            justify,
            wrap_style,
        } = *constraints;
        Self {
            max_width: max_width_px.map(canonical_f32_bits),
            max_height: max_height_px.map(canonical_f32_bits),
            wrap: match wrap {
                None => 0,
                Some(TextWrapBreak::Word) => 1,
                Some(TextWrapBreak::WordOrGlyph) => 2,
                Some(TextWrapBreak::Glyph) => 3,
            },
            word_break: match word_break {
                WordBreakSpec::Normal => 0,
                WordBreakSpec::BreakAll => 1,
                WordBreakSpec::BreakWord => 2,
            },
            line_break: match line_break {
                LineBreakSpec::Auto => 0,
                LineBreakSpec::Normal => 1,
                LineBreakSpec::Anywhere => 2,
                LineBreakSpec::Strict => 3,
                LineBreakSpec::Loose => 4,
            },
            max_lines,
            ellipsis,
            preserve_lines,
            base_direction: match base_direction {
                DirSpec::Ltr => 0,
                DirSpec::Rtl => 1,
            },
            align: match align {
                TextAlignSpec::Start => 0,
                TextAlignSpec::Center => 1,
                TextAlignSpec::End => 2,
                TextAlignSpec::Left => 3,
                TextAlignSpec::Right => 4,
                TextAlignSpec::Justify => 5,
            },
            writing_mode: match writing_mode {
                WritingModeSpec::HorizontalTb => 0,
                WritingModeSpec::VerticalRl => 1,
                WritingModeSpec::VerticalLr => 2,
            },
            text_orientation: match text_orientation {
                TextOrientationSpec::Mixed => 0,
                TextOrientationSpec::Upright => 1,
                TextOrientationSpec::Sideways => 2,
            },
            tab_width,
            scale: canonical_f32_bits(scale.px_per_logical),
            spacing_trim: match spacing_trim {
                TextSpacingTrimSpec::SpaceAll => 0,
                TextSpacingTrimSpec::Normal => 1,
                TextSpacingTrimSpec::TrimBoth => 2,
                TextSpacingTrimSpec::Auto => 3,
            },
            autospace: match autospace {
                TextAutospaceSpec::NoAutospace => 0,
                TextAutospaceSpec::Normal => 1,
            },
            justify: match justify {
                TextJustifySpec::Auto => 0,
                TextJustifySpec::None => 1,
                TextJustifySpec::InterWord => 2,
                TextJustifySpec::InterCharacter => 3,
            },
            wrap_style: match wrap_style {
                TextWrapStyleSpec::Auto => 0,
                TextWrapStyleSpec::Pretty => 1,
            },
        }
    }
}

#[derive(Debug, Clone)]
pub(crate) struct LayoutKey {
    shaped: Arc<ShapedText>,
    /// The revision the requesting source is at. See the module comment.
    revision: TextRevision,
    /// The shaped ellipsis this layout may truncate with. A different ellipsis
    /// is a different layout, and no ellipsis at all is a third.
    ellipsis: Option<Arc<ShapedText>>,
    kind: TextKind,
    constraints: ConstraintsKey,
    /// Ascent, descent, line gap and line box height of the strut.
    strut: Option<[u32; 4]>,
    /// Resolved line box height per shaped run, in physical px. Line height is
    /// not a shaping input, so it cannot come in through the shaped identity.
    run_line_heights: Vec<u32>,
    empty_line_height: u32,
    /// Every inline object's offset and box (width, ascent, descent). Not a
    /// shaping input — objects shape as placeholders — so it cannot come in
    /// through the shaped identity either.
    objects: Vec<(usize, [u32; 7])>,
    /// Each ruby's base range and its shaped annotation, held so the pointer
    /// it is compared by stays valid.
    rubies: Vec<(std::ops::Range<usize>, Arc<ShapedText>)>,
    /// Each object label's offset and shaped text, held like the rubies.
    labels: Vec<(usize, Arc<ShapedText>)>,
}

impl LayoutKey {
    #[expect(
        clippy::too_many_arguments,
        reason = "every layout input is a key field"
    )]
    pub fn new(
        shaped: &Arc<ShapedText>,
        revision: TextRevision,
        ellipsis: Option<&Arc<ShapedText>>,
        kind: TextKind,
        constraints: &TextConstraints,
        strut: Option<LineStrut>,
        run_line_heights: &[f32],
        empty_line_height: f32,
        objects: &[crate::source::InlineObject],
        rubies: Vec<(std::ops::Range<usize>, Arc<ShapedText>)>,
        labels: Vec<(usize, Arc<ShapedText>)>,
    ) -> Self {
        Self {
            shaped: Arc::clone(shaped),
            revision,
            ellipsis: ellipsis.map(Arc::clone),
            kind,
            constraints: ConstraintsKey::new(constraints),
            strut: strut.map(|strut| {
                [
                    canonical_f32_bits(strut.metrics.ascent_px),
                    canonical_f32_bits(strut.metrics.descent_px),
                    canonical_f32_bits(strut.metrics.line_gap_px),
                    canonical_f32_bits(strut.line_height_px),
                ]
            }),
            run_line_heights: run_line_heights
                .iter()
                .copied()
                .map(canonical_f32_bits)
                .collect(),
            empty_line_height: canonical_f32_bits(empty_line_height),
            objects: objects
                .iter()
                .map(|object| {
                    let envelope = object.metrics.envelope;
                    (
                        object.offset,
                        [
                            canonical_f32_bits(object.metrics.width_px),
                            canonical_f32_bits(object.metrics.ascent_px),
                            canonical_f32_bits(object.metrics.descent_px),
                            u32::from(envelope.is_some()),
                            envelope.map_or(0, |envelope| canonical_f32_bits(envelope.gap_px)),
                            envelope
                                .map_or(0, |envelope| canonical_f32_bits(envelope.gap_shrink_px)),
                            envelope.map_or(0, |envelope| {
                                u32::from(envelope.break_before)
                                    | (u32::from(envelope.break_after) << 1)
                            }),
                        ],
                    )
                })
                .collect(),
            rubies,
            labels,
        }
    }

    /// Identity of the shaped runs this layout was built from.
    pub fn shaped_identity(&self) -> usize {
        Arc::as_ptr(&self.shaped) as *const u8 as usize
    }

    /// Bytes the key itself retains. The shaped text is shared with the shape
    /// cache, which already charges for it, so it is not counted twice.
    pub fn retained_bytes(&self) -> usize {
        std::mem::size_of::<Self>()
            + self.run_line_heights.capacity() * std::mem::size_of::<u32>()
            + self.objects.capacity() * std::mem::size_of::<(usize, [u32; 7])>()
    }
}

impl PartialEq for LayoutKey {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.shaped, &other.shaped)
            && self.revision == other.revision
            && match (&self.ellipsis, &other.ellipsis) {
                (None, None) => true,
                (Some(left), Some(right)) => Arc::ptr_eq(left, right),
                _ => false,
            }
            && self.kind == other.kind
            && self.constraints == other.constraints
            && self.strut == other.strut
            && self.run_line_heights == other.run_line_heights
            && self.empty_line_height == other.empty_line_height
            && self.objects == other.objects
            && self.rubies.len() == other.rubies.len()
            && self
                .rubies
                .iter()
                .zip(&other.rubies)
                .all(|((left, a), (right, b))| left == right && Arc::ptr_eq(a, b))
            && self.labels.len() == other.labels.len()
            && self
                .labels
                .iter()
                .zip(&other.labels)
                .all(|((left, a), (right, b))| left == right && Arc::ptr_eq(a, b))
    }
}

impl Eq for LayoutKey {}

impl Hash for LayoutKey {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.shaped_identity().hash(state);
        self.revision.hash(state);
        self.ellipsis
            .as_ref()
            .map(|shaped| Arc::as_ptr(shaped) as *const u8 as usize)
            .hash(state);
        self.kind.hash(state);
        self.constraints.hash(state);
        self.strut.hash(state);
        self.run_line_heights.hash(state);
        self.empty_line_height.hash(state);
        self.objects.hash(state);
        for (range, shaped) in &self.rubies {
            range.hash(state);
            (Arc::as_ptr(shaped) as *const u8 as usize).hash(state);
        }
        for (offset, shaped) in &self.labels {
            offset.hash(state);
            (Arc::as_ptr(shaped) as *const u8 as usize).hash(state);
        }
    }
}
