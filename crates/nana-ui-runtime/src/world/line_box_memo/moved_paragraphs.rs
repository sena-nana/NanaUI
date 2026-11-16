//! A paragraph that only moves keeps everything under it.
//!
//! A page of block paragraphs, each an inline formatting context of text
//! runs and an inline-block atom, under a header whose height the edits
//! change: every paragraph below it moves and none changes. Its line boxes
//! and runs are measured from its own origin, so the pass writes its box and
//! leaves its subtree as it was (`placement::keeps_moved_subtree`), instead
//! of packing its line boxes again. Some paragraphs also hold an
//! out-of-flow child: one whose static position hangs from the page, one hung
//! from a positioned ancestor by its insets, a fixed one, and one whose
//! containing block is the paragraph itself. The ones whose child reads
//! where the paragraph stands keep their body.
//!
//! Every step is compared with the same inputs laid out cold
//! (`assert_matches_cold`), and the per-pass guard requires every box under
//! a kept subtree to be the full layout's to the bit
//! (`verify::note_kept_subtree`).
#![cfg(test)]

use nana_ui_core::{DisplaySpec, LayoutStyle, LengthSpec, PositionSpec};

use super::super::reflow_oracle::{
    Builder, assert_matches_cold, bundled_face_shaper, product_frame,
};
use crate::{AppContext, DocumentId, LayoutViewport, MutationQueue, NanaTextEngineShaper};
use crate::{NodeStyle, StableNodeId, TextContent};

fn viewport() -> LayoutViewport {
    LayoutViewport::new(800.0, 600.0)
}

fn document() -> DocumentId {
    DocumentId::new(1).unwrap()
}

fn style(layout: LayoutStyle) -> NodeStyle {
    super::super::reflow_oracle::styled(layout)
}

fn header(height: f32) -> LayoutStyle {
    LayoutStyle {
        height: Some(LengthSpec::Px(height)),
        ..LayoutStyle::default()
    }
}

/// What hangs out of a paragraph's flow, if anything.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Hung {
    None,
    /// Absolute, both insets auto, no positioned ancestor: its static
    /// position on the page reads where the paragraph stands.
    StaticOnPage,
    /// Absolute, hung from the positioned page by its insets.
    InsetFromPage,
    /// Absolute, both insets auto, from a positioned ancestor above the
    /// paragraph: its static position reads the paragraph's offset in it.
    StaticInBlock,
    Fixed,
    /// Absolute in a positioned paragraph: measured from the paragraph.
    InParagraph,
}

const PARAGRAPHS: [Hung; 8] = [
    Hung::None,
    Hung::StaticOnPage,
    Hung::None,
    Hung::InsetFromPage,
    Hung::Fixed,
    Hung::None,
    Hung::InParagraph,
    Hung::StaticInBlock,
];

/// What the edits address: the boxes whose height moves the paragraphs, and
/// the second run of the first paragraph.
struct Ids {
    heads: Vec<StableNodeId>,
    run: StableNodeId,
}

/// The page, laid out once: a header `height` tall, then the paragraphs.
/// The paragraph whose child hangs from a positioned block stands in that
/// block below a box as tall as the header, so it moves within the block
/// too. Ids follow the same order whatever the inputs are, so a warm page
/// and a cold one agree on them.
fn page(height: f32, run: &str, positioned_page: bool) -> (AppContext, NanaTextEngineShaper, Ids) {
    let (mut b, page) = Builder::page(document(), 1, 400.0);
    if positioned_page {
        b.queue.set_style(
            page,
            style(LayoutStyle {
                position: PositionSpec::Relative,
                ..super::super::reflow_oracle::column(Some(400.0))
            }),
        );
    }
    let mut heads = vec![b.element(page, header(height))];
    let mut edited = None;
    for hung in PARAGRAPHS {
        let holder = if hung == Hung::StaticInBlock {
            let holder = b.element(
                page,
                LayoutStyle {
                    position: PositionSpec::Relative,
                    ..LayoutStyle::default()
                },
            );
            heads.push(b.element(holder, header(height)));
            holder
        } else {
            page
        };
        let paragraph = b.element(
            holder,
            LayoutStyle {
                width: Some(LengthSpec::Px(160.0)),
                position: if hung == Hung::InParagraph {
                    PositionSpec::Relative
                } else {
                    PositionSpec::Static
                },
                ..LayoutStyle::default()
            },
        );
        b.label(paragraph, "The quick brown fox jumps over");
        b.element(
            paragraph,
            LayoutStyle {
                display: Some(DisplaySpec::InlineFlex),
                width: Some(LengthSpec::Px(24.0)),
                height: Some(LengthSpec::Px(16.0)),
                ..LayoutStyle::default()
            },
        );
        // The first paragraph's second run is the one the edits rewrap.
        let second = b.label(
            paragraph,
            if edited.is_none() {
                run
            } else {
                "the lazy dog"
            },
        );
        edited.get_or_insert(second);
        let (position, inset) = match hung {
            Hung::None => continue,
            Hung::StaticOnPage | Hung::StaticInBlock => (PositionSpec::Absolute, None),
            Hung::InsetFromPage | Hung::InParagraph => (PositionSpec::Absolute, Some(6.0)),
            Hung::Fixed => (PositionSpec::Fixed, None),
        };
        b.element(
            paragraph,
            LayoutStyle {
                position,
                offset_top: inset.map(LengthSpec::Px),
                offset_left: inset.map(LengthSpec::Px),
                width: Some(LengthSpec::Px(10.0)),
                height: Some(LengthSpec::Px(10.0)),
                ..LayoutStyle::default()
            },
        );
    }
    let queue = std::mem::take(&mut b.queue);
    let mut context = AppContext::new();
    context.commit_mutations(queue).unwrap();
    let mut shaper = bundled_face_shaper();
    product_frame(&mut context, document(), viewport(), &mut shaper);
    let run = edited.unwrap();
    (context, shaper, Ids { heads, run })
}

fn step(
    warm: &mut AppContext,
    shaper: &mut NanaTextEngineShaper,
    cold: (f32, &str, bool),
    edit: impl FnOnce(&mut MutationQueue),
) -> usize {
    let mut queue = MutationQueue::new();
    edit(&mut queue);
    warm.commit_mutations(queue).unwrap();
    product_frame(warm, document(), viewport(), shaper);
    let kept = warm.layout_execution_stats(document()).moved_subtrees_kept;
    let (mut cold, _, _) = page(cold.0, cold.1, cold.2);
    assert_matches_cold(warm, &mut cold, document());
    kept
}

/// The plain paragraphs below a header that grows and shrinks keep their
/// subtrees; the ones whose out-of-flow child reads where they stand are
/// laid out again; and every box equals a cold layout's.
#[test]
fn a_moved_paragraph_keeps_its_line_boxes() {
    for positioned_page in [false, true] {
        let (mut warm, mut shaper, ids) = page(20.0, "the lazy dog", positioned_page);
        for height in [44.0, 20.0, 31.5] {
            let kept = step(
                &mut warm,
                &mut shaper,
                (height, "the lazy dog", positioned_page),
                |queue| {
                    for &head in &ids.heads {
                        queue.set_style(head, style(header(height)));
                    }
                },
            );
            // The three plain paragraphs and the one whose child is measured
            // from it. A child hung from the page or from a block above
            // (its static position or its insets) is placed by the body, as
            // is a fixed one.
            assert_eq!(
                kept, 4,
                "height {height}, positioned page {positioned_page}"
            );
        }
    }
}

/// A run's text that rewraps its paragraph moves the ones below it, which
/// keep their subtrees; the edited one does not.
#[test]
fn paragraphs_below_a_rewrapped_one_keep_their_line_boxes() {
    let (mut warm, mut shaper, ids) = page(20.0, "the lazy dog", false);
    for value in [
        "the lazy dog, a sleepy cat and a very slow tortoise",
        "the lazy dog",
    ] {
        let kept = step(&mut warm, &mut shaper, (20.0, value, false), |queue| {
            queue.set_text(ids.run, TextContent::new(value));
        });
        // The two plain paragraphs below it, the one whose child is measured
        // from it, and the positioned block: the paragraph in it stays where
        // it was in it.
        assert_eq!(kept, 4, "{value:?}");
    }
}
