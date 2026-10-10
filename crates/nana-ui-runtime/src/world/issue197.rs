//! Issue #197: a node's identity, how it takes part in its parent's
//! formatting context, and the context it establishes are three things.
//!
//! One container holds a labelled inline-block, an inline text node, an
//! inline-block image and a grid. Its display moves Flex → Grid → Block (an
//! inline context, since its children are inline-level) → Flex.
//!
//! - Gate A: each step records exactly one context transition, on the
//!   container. Every child keeps its id; nothing spawns or despawns. Each
//!   child's participation follows the parent's context; the grid keeps
//!   establishing a grid whatever it takes part in. The result equals a cold
//!   layout of the same inputs.
//! - Gate B: a static frame, a paint-only edit and an equivalent intent
//!   record no transition and leave the context generation where it was.

#![cfg(test)]

use nana_ui_core::{DisplaySpec, FlexDirection, GridTrack, LayoutStyle, LengthSpec};

use super::issue263::image_layout;
use super::reflow_oracle::{
    Builder, assert_matches_cold, bundled_face_shaper, product_frame, styled,
};
use super::{DocumentId, StableNodeId, UiWorld};
use crate::{
    AppContext, FormattingContextKind as Ctx, LayoutContentKind, LayoutViewport, MutationQueue,
    NanaTextEngineShaper, ParticipationKind as Part, TextContent,
};

fn viewport() -> LayoutViewport {
    LayoutViewport::new(800.0, 600.0)
}

fn container(display: DisplaySpec) -> LayoutStyle {
    LayoutStyle {
        display: Some(display),
        width: Some(LengthSpec::Px(600.0)),
        ..LayoutStyle::default()
    }
}

fn inline_block(width: f32, height: f32) -> LayoutStyle {
    LayoutStyle {
        display: Some(DisplaySpec::InlineBlock),
        width: Some(LengthSpec::Px(width)),
        height: Some(LengthSpec::Px(height)),
        ..LayoutStyle::default()
    }
}

struct Parts {
    container: StableNodeId,
    /// An inline-block leaf with a label: a control.
    button: StableNodeId,
    /// A text node that is inline-level.
    text: StableNodeId,
    image: StableNodeId,
    grid: StableNodeId,
}

impl Parts {
    fn children(&self) -> [StableNodeId; 4] {
        [self.button, self.text, self.image, self.grid]
    }
}

struct Page {
    context: AppContext,
    document: DocumentId,
    shaper: NanaTextEngineShaper,
    parts: Parts,
}

impl Page {
    fn new(display: DisplaySpec) -> Self {
        let document = DocumentId::new(1).unwrap();
        let (mut b, page) = Builder::page(document, 1, 800.0);
        let container = b.element(page, container(display));
        let button = b.element(container, inline_block(80.0, 24.0));
        b.queue.set_text(
            button,
            TextContent {
                value: "保存".into(),
            },
        );
        let text = b.label(container, "CPU 42%");
        b.queue.set_style(
            text,
            styled(LayoutStyle {
                display: Some(DisplaySpec::Inline),
                ..LayoutStyle::default()
            }),
        );
        let mut image = image_layout("icon", Some(24.0), Some(24.0));
        image.display = Some(DisplaySpec::InlineBlock);
        let image = b.element(container, image);
        let grid = b.element(
            container,
            LayoutStyle {
                display: Some(DisplaySpec::Grid),
                grid_columns: Some(vec![GridTrack::Px(40.0), GridTrack::Px(40.0)]),
                ..LayoutStyle::default()
            },
        );
        for _ in 0..2 {
            b.element(
                grid,
                LayoutStyle {
                    height: Some(LengthSpec::Px(20.0)),
                    ..LayoutStyle::default()
                },
            );
        }
        let mut context = AppContext::new();
        context.commit_mutations(b.queue).unwrap();
        let mut page = Self {
            context,
            document,
            shaper: bundled_face_shaper(),
            parts: Parts {
                container,
                button,
                text,
                image,
                grid,
            },
        };
        page.frame();
        page
    }

    fn world(&self) -> &UiWorld {
        self.context.world()
    }

    fn frame(&mut self) -> nana_ui_core::WorkCounters {
        product_frame(
            &mut self.context,
            self.document,
            viewport(),
            &mut self.shaper,
        )
    }

    fn set_layout(&mut self, id: StableNodeId, layout: LayoutStyle) {
        let mut queue = MutationQueue::new();
        queue.set_style(id, styled(layout));
        self.context.commit_mutations(queue).unwrap();
    }

    fn established(&self, id: StableNodeId) -> Option<Ctx> {
        self.world().layout_node(id).unwrap().established
    }

    fn participation(&self, id: StableNodeId) -> Option<Part> {
        self.world().layout_node(id).unwrap().participation
    }

    fn context_generation(&self, id: StableNodeId) -> u64 {
        self.world().layout_node(id).unwrap().context_generation
    }
}

/// Gate A. The same children through Flex, Grid, an inline context and back.
#[test]
fn issue197_a_context_transition_keeps_every_child() {
    let mut page = Page::new(DisplaySpec::Flex);
    let parts = &page.parts;
    let (container, button, text, image, grid) = (
        parts.container,
        parts.button,
        parts.text,
        parts.image,
        parts.grid,
    );
    let children = parts.children();
    let nodes = page.world().len();
    let child_list = page.world().child_ids(container).to_vec();
    assert_eq!(child_list, children);

    // Content is the node's own, whatever its component.
    let content = |page: &Page, id| page.world().layout_node(id).unwrap().content;
    assert_eq!(content(&page, button), LayoutContentKind::Text);
    assert_eq!(content(&page, text), LayoutContentKind::Text);
    assert_eq!(content(&page, image), LayoutContentKind::Replaced);
    assert_eq!(content(&page, grid), LayoutContentKind::Children);
    // A labelled control runs its own lines; they are not layout nodes.
    assert_eq!(page.established(button), Some(Ctx::Inline));
    assert_eq!(page.established(image), None);

    let steps = [
        (DisplaySpec::Grid, Ctx::Grid, [Part::GridItem; 4]),
        (
            DisplaySpec::Block,
            Ctx::Inline,
            [
                Part::AtomicInline,
                Part::NativeText,
                Part::AtomicInline,
                // A grid is block-level: it breaks the lines around it.
                Part::FlowItem,
            ],
        ),
        (DisplaySpec::Flex, Ctx::Flex, [Part::FlexItem; 4]),
    ];
    assert_eq!(page.established(container), Some(Ctx::Flex));
    for child in children {
        assert_eq!(page.participation(child), Some(Part::FlexItem), "{child:?}");
    }
    for (display, context, participation) in steps {
        let generation = page.context_generation(container);
        page.set_layout(container, self::container(display));
        let counters = page.frame();
        assert_eq!(counters.layout_context_transitions, 1, "{display:?}");
        assert_eq!(counters.entities_spawned, 0, "{display:?}");
        assert_eq!(counters.entities_despawned, 0, "{display:?}");
        assert_eq!(page.world().len(), nodes, "{display:?}");
        assert_eq!(page.world().child_ids(container), child_list, "{display:?}");
        assert_eq!(page.established(container), Some(context), "{display:?}");
        assert_eq!(page.context_generation(container), generation + 1);
        for (child, expected) in children.into_iter().zip(participation) {
            let view = page.world().layout_node(child).unwrap();
            assert_eq!(view.parent_context, Some(context), "{display:?} {child:?}");
            assert_eq!(view.participation, Some(expected), "{display:?} {child:?}");
        }
        // The nested grid keeps its own context whatever it takes part in.
        assert_eq!(page.established(grid), Some(Ctx::Grid), "{display:?}");
        assert_eq!(page.established(button), Some(Ctx::Inline), "{display:?}");
        let result = page.world().layout_result(container).unwrap();
        assert_eq!(result.formatting_context, Some(context));
        assert!(
            result
                .fragments
                .iter()
                .filter(|fragment| fragment.node.is_some())
                .all(|fragment| fragment.kind == context.child_fragment_kind()),
            "{display:?}: {:?}",
            result.fragments
        );

        let mut cold = Page::new(display);
        assert_matches_cold(&mut page.context, &mut cold.context, page.document);
    }
}

/// Gate B. Nothing that leaves the context alone records a transition.
#[test]
fn issue197_static_paint_and_equivalent_frames_keep_the_context() {
    let mut page = Page::new(DisplaySpec::Block);
    let container = page.parts.container;
    assert_eq!(page.established(container), Some(Ctx::Inline));
    let generation = page.context_generation(container);

    let counters = page.frame();
    assert_eq!(counters.layout_context_transitions, 0, "static");

    let mut painted = self::container(DisplaySpec::Block);
    painted.opacity = Some(0.5);
    page.set_layout(container, painted);
    let counters = page.frame();
    assert_eq!(counters.layout_context_transitions, 0, "paint-only");

    // `None` and the implicit column are one intent.
    let mut column = self::container(DisplaySpec::Block);
    column.opacity = Some(0.5);
    column.direction = Some(FlexDirection::Column);
    page.set_layout(container, column);
    let counters = page.frame();
    assert_eq!(counters.layout_context_transitions, 0, "equivalent");

    assert_eq!(page.context_generation(container), generation);
    assert_eq!(page.established(container), Some(Ctx::Inline));
}
