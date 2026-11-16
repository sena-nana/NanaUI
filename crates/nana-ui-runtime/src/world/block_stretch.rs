//! A container that leaves `display` unset is a block container, the same
//! as `display: block` (CSS 2.1 §10.3.3): it has no `align-items`, and a
//! block-level child whose width is `auto` fills it less its margins. A
//! child that names `align-self`, has a width of its own, is replaced, or is
//! in an orthogonal writing mode keeps its own width; a row and every flex
//! container align by `align_items`.
//!
//! The gate lays out one 400 px container holding a child with 10 / 30 px
//! margins around a 50 px box, then edits the container's `display`,
//! `direction`, `align_items` and width and the child's `align-self`, width
//! and writing mode one at a time. Every step meets the box CSS gives the
//! child and a cold layout of the same inputs.

#![cfg(test)]

use nana_ui_core::{
    AlignSpec, DisplaySpec, FlexDirection, LayoutStyle, LengthSpec, WritingModeSpec,
};

use super::reflow_oracle::{
    Builder, assert_matches_cold, bundled_face_shaper, product_frame, styled,
};
use crate::{AppContext, DocumentId, LayoutViewport, MutationQueue, StableNodeId};

const MARGIN_LEFT: f32 = 10.0;
const MARGIN_RIGHT: f32 = 30.0;
const CONTENT: f32 = 50.0;

#[derive(Clone, Copy, Debug)]
struct Spec {
    display: Option<DisplaySpec>,
    direction: Option<FlexDirection>,
    align_items: AlignSpec,
    width: f32,
    align_self: Option<AlignSpec>,
    child_width: Option<LengthSpec>,
    orthogonal: bool,
}

impl Spec {
    /// The child's width and its left edge in the container, as CSS has
    /// them.
    fn expected(self) -> (f32, f32) {
        if self.direction == Some(FlexDirection::Row) {
            // Laid along the row: no stretch on the main axis.
            return (CONTENT, MARGIN_LEFT);
        }
        let block = self.display.is_none();
        let align = self.align_self.unwrap_or(if block {
            AlignSpec::Stretch
        } else {
            self.align_items
        });
        let available = self.width - MARGIN_LEFT - MARGIN_RIGHT;
        let keeps_content =
            self.child_width.is_some() || (block && self.align_self.is_none() && self.orthogonal);
        let width = if align == AlignSpec::Stretch && !keeps_content {
            available
        } else {
            CONTENT
        };
        let x = match align {
            AlignSpec::Center => MARGIN_LEFT + (available - width) / 2.0,
            AlignSpec::End => self.width - MARGIN_RIGHT - width,
            _ => MARGIN_LEFT,
        };
        (width, x)
    }
}

fn viewport() -> LayoutViewport {
    LayoutViewport::new(800.0, 600.0)
}

fn container_style(spec: Spec) -> LayoutStyle {
    LayoutStyle {
        display: spec.display,
        direction: spec.direction,
        align_items: spec.align_items,
        width: Some(LengthSpec::Px(spec.width)),
        ..LayoutStyle::default()
    }
}

fn child_style(spec: Spec) -> LayoutStyle {
    LayoutStyle {
        align_self: spec.align_self,
        width: spec.child_width,
        margin_left: Some(LengthSpec::Px(MARGIN_LEFT)),
        margin_right: Some(LengthSpec::Px(MARGIN_RIGHT)),
        writing_mode: spec.orthogonal.then_some(WritingModeSpec::VerticalRl),
        ..LayoutStyle::default()
    }
}

struct Page {
    context: AppContext,
    container: StableNodeId,
    child: StableNodeId,
}

fn page(spec: Spec) -> Page {
    let document = DocumentId::new(1).unwrap();
    let (mut b, page) = Builder::page(document, 1, 800.0);
    let container = b.element(page, container_style(spec));
    let child = b.element(container, child_style(spec));
    b.element(
        child,
        LayoutStyle {
            width: Some(LengthSpec::Px(CONTENT)),
            height: Some(LengthSpec::Px(10.0)),
            ..LayoutStyle::default()
        },
    );
    let mut context = AppContext::new();
    context.commit_mutations(b.queue).unwrap();
    product_frame(
        &mut context,
        document,
        viewport(),
        &mut bundled_face_shaper(),
    );
    Page {
        context,
        container,
        child,
    }
}

fn assert_child(page: &Page, spec: Spec) {
    let world = page.context.world();
    let container = world.layout_box(page.container).unwrap();
    let child = world.layout_box(page.child).unwrap();
    let (width, x) = spec.expected();
    assert!(
        (child.width - width).abs() < 0.01 && (child.x - container.x - x).abs() < 0.01,
        "{spec:?}: expected width {width} at {x}, the child is {child:?} in {container:?}"
    );
}

#[test]
fn an_unset_display_stacks_as_a_block_through_every_edit() {
    let document = DocumentId::new(1).unwrap();
    let mut spec = Spec {
        display: None,
        direction: None,
        align_items: AlignSpec::Start,
        width: 400.0,
        align_self: None,
        child_width: None,
        orthogonal: false,
    };
    let mut warm = page(spec);
    assert_child(&warm, spec);
    let mut shaper = bundled_face_shaper();
    let steps: [fn(&mut Spec); 17] = [
        // A block has no `align-items`.
        |spec| spec.align_items = AlignSpec::Center,
        |spec| spec.align_items = AlignSpec::End,
        // `align-self` the child names still holds.
        |spec| spec.align_self = Some(AlignSpec::Center),
        |spec| spec.align_self = None,
        // Laid along a row, and back to the block axis.
        |spec| spec.direction = Some(FlexDirection::Row),
        |spec| spec.direction = Some(FlexDirection::Column),
        // A flex column aligns by `align_items`; an unset container does not.
        |spec| spec.display = Some(DisplaySpec::Flex),
        |spec| spec.align_items = AlignSpec::Start,
        |spec| spec.display = None,
        |spec| spec.direction = None,
        // The fill follows the container's width.
        |spec| spec.width = 300.0,
        // An orthogonal child keeps the block size its content gives it.
        |spec| spec.orthogonal = true,
        |spec| spec.orthogonal = false,
        // A width of its own is kept.
        |spec| spec.child_width = Some(LengthSpec::Shrink),
        |spec| spec.child_width = None,
        |spec| spec.width = 400.0,
        |spec| spec.align_self = Some(AlignSpec::End),
    ];
    for step in steps {
        step(&mut spec);
        let mut queue = MutationQueue::new();
        queue.set_style(warm.container, styled(container_style(spec)));
        queue.set_style(warm.child, styled(child_style(spec)));
        warm.context.commit_mutations(queue).unwrap();
        product_frame(&mut warm.context, document, viewport(), &mut shaper);
        assert_child(&warm, spec);
        let mut cold = page(spec);
        assert_child(&cold, spec);
        assert_matches_cold(&mut warm.context, &mut cold.context, document);
    }
}
