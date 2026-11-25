//! What a restyle learns from comparing two resolved layouts, remembered by
//! the pair of `Arc`s.
//!
//! Layouts are interned (`LayoutInterner`), so nodes built alike hold the
//! same `Arc`, and every new node starts from the shared default: creating a
//! page of rows compares the same two layouts once per node. The answer is a
//! pure function of the two layouts, so a pair seen recently answers from
//! here. The memo holds both `Arc`s, which keeps their addresses from being
//! reused and their contents from being written in place (`Arc::make_mut`
//! copies a shared layout), so a pointer match is a match by value.

use std::sync::Arc;

use nana_ui_core::{LayoutStyle, LayoutStyleChange};

use crate::node_space::NodeSpace;

/// The comparisons [`super::UiWorld::restyle`] makes between a node's
/// previous resolved layout and its next one.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct LayoutPairFacts {
    /// `changed_fields`, or nothing for the very same `Arc`.
    pub changed: LayoutStyleChange,
    /// Position or either overflow axis moved: sticky bindings follow.
    pub sticky_inputs: bool,
    /// An inherited text or writing-direction field moved.
    pub inherited_text: bool,
    /// An inherited paint field of the layout moved (the foreground is the
    /// presentation's and compared apart).
    pub inherited_paint: bool,
    pub opacity: bool,
    pub paint_visibility: bool,
    pub pointer_events: bool,
    pub cursor: bool,
    pub user_select: bool,
    pub omits_box: bool,
    pub content_image: bool,
    /// The previous space keeps the next one's (`NodeSpace::keeps_space`).
    pub keeps_space: bool,
    /// `same_text_constraint_inputs` over the two layouts.
    pub same_text_constraints: bool,
}

impl LayoutPairFacts {
    pub(super) fn compute(previous: &Arc<LayoutStyle>, next: &Arc<LayoutStyle>) -> Self {
        let (p, n) = (previous.as_ref(), next.as_ref());
        let text_ext_diff = !p.text_ext.ptr_eq(&n.text_ext);
        Self {
            changed: if Arc::ptr_eq(previous, next) {
                LayoutStyleChange::NONE
            } else {
                p.changed_fields(n)
            },
            sticky_inputs: p.position != n.position
                || p.overflow_x != n.overflow_x
                || p.overflow_y != n.overflow_y,
            inherited_text: p.font_size != n.font_size
                || p.font_weight != n.font_weight
                || p.font_italic != n.font_italic
                || p.line_height != n.line_height
                // Writing mode and direction inherit into layout, not just
                // text: a descendant container lays out along them too, and
                // the INPUT bit re-projects the accessibility nodes that
                // report which way their text reads.
                || p.dir != n.dir
                || p.writing_mode != n.writing_mode
                || (text_ext_diff && {
                    let (pt, nt) = (&*p.text_ext, &*n.text_ext);
                    pt.font_family != nt.font_family
                        || pt.letter_spacing != nt.letter_spacing
                        || pt.font_features != nt.font_features
                        || pt.font_variation_settings != nt.font_variation_settings
                        || pt.font_kerning != nt.font_kerning
                        || pt.word_break != nt.word_break
                        || pt.line_break != nt.line_break
                        || pt.hyphens != nt.hyphens
                        || pt.dominant_baseline != nt.dominant_baseline
                        || pt.text_typography != nt.text_typography
                        || pt.text_emphasis_style != nt.text_emphasis_style
                        || pt.text_emphasis_position != nt.text_emphasis_position
                        || pt.text_orientation != nt.text_orientation
                }),
            inherited_paint: (text_ext_diff
                && p.text_ext.text_emphasis_color != n.text_ext.text_emphasis_color)
                || p.paint_colors.color != n.paint_colors.color
                || p.paint_colors.selection_background != n.paint_colors.selection_background
                || p.paint_colors.selection_color != n.paint_colors.selection_color,
            opacity: p.opacity != n.opacity,
            paint_visibility: p.paint.visibility != n.paint.visibility,
            pointer_events: p.pointer_events != n.pointer_events,
            cursor: p.cursor != n.cursor,
            user_select: p.user_select != n.user_select,
            omits_box: p.omits_box() != n.omits_box(),
            content_image: p.paint.content_image != n.paint.content_image,
            keeps_space: NodeSpace::of_style(p).keeps_space(NodeSpace::of_style(n)),
            same_text_constraints: super::text::same_text_constraint_layouts(previous, next),
        }
    }
}

/// How many pairs a world remembers. Building a page alternates among a few
/// kinds of node (a row, its cells, their leaves), each from the default.
const PAIRS: usize = 8;

/// Recent pairs of resolved layouts and what comparing them found.
#[derive(Default)]
pub(super) struct LayoutPairMemo {
    pairs: [Option<(
        Arc<LayoutStyle>,
        Arc<LayoutStyle>,
        LayoutPairFacts,
        Option<bool>,
    )>; PAIRS],
    next: usize,
}

impl LayoutPairMemo {
    /// What comparing `previous` with `next` finds.
    pub(super) fn facts(
        &mut self,
        previous: &Arc<LayoutStyle>,
        next: &Arc<LayoutStyle>,
    ) -> LayoutPairFacts {
        let kept = self
            .pairs
            .iter()
            .flatten()
            .find(|(p, n, _, _)| Arc::ptr_eq(p, previous) && Arc::ptr_eq(n, next));
        if let Some((_, _, facts, _)) = kept {
            #[cfg(any(test, feature = "layout-verify"))]
            assert_eq!(
                *facts,
                LayoutPairFacts::compute(previous, next),
                "a remembered layout pair answers as comparing it again does"
            );
            return *facts;
        }
        let facts = LayoutPairFacts::compute(previous, next);
        self.pairs[self.next] = Some((Arc::clone(previous), Arc::clone(next), facts, None));
        self.next = (self.next + 1) % PAIRS;
        facts
    }

    /// Whether `previous` and `next` are equal by value. A remembered pair
    /// keeps the answer; a pair not remembered is compared and left out, so
    /// a style write does not push out the pairs restyles reuse.
    pub(super) fn equal(&mut self, previous: &Arc<LayoutStyle>, next: &Arc<LayoutStyle>) -> bool {
        let kept = self
            .pairs
            .iter_mut()
            .flatten()
            .find(|(p, n, _, _)| Arc::ptr_eq(p, previous) && Arc::ptr_eq(n, next));
        match kept {
            Some((_, _, _, Some(equal))) => {
                #[cfg(any(test, feature = "layout-verify"))]
                assert_eq!(*equal, **previous == **next);
                *equal
            }
            Some((_, _, _, slot)) => *slot.insert(**previous == **next),
            None => **previous == **next,
        }
    }
}
