//! The clip chain (#201) through a real frame: the scene paints each box
//! where accessibility says it is and cuts it where accessibility says it is
//! cut, and the pointer finds it in that same visible part, at every scroll
//! position and without a layout pass.
//!
//! A positioned page holds a static scroller `outer`. In it, a positioned
//! card holds a static scroller `inner` with a row in flow, an absolute box
//! whose block is the card and a fixed box; after the card, a static
//! clipping box holds an absolute box whose block is the page.

use std::sync::Arc;

use nana_ui_core::{FlexDirection, LayoutStyle, LengthSpec, OverflowSpec, PositionSpec};
use nana_ui_runtime::{
    ComputedStyle, InteractionState, LayoutBox, LayoutViewport, MutationQueue, NodeKind, NodeStyle,
    ScrollOffset, StableNodeId, TextContent, TextMetrics,
};

use super::*;
use crate::{AffineTransform, RenderOperation, SceneRect};

struct NoText;

impl nana_ui_runtime::TextShaper for NoText {
    fn shape(
        &mut self,
        _id: StableNodeId,
        _text: &TextContent,
        _style: &ComputedStyle,
        _constraints: nana_ui_runtime::TextShapeConstraints,
    ) -> TextMetrics {
        TextMetrics {
            width: 0.0,
            height: 0.0,
            ascent: None,
        }
    }
}

fn sized(width: f32, height: f32) -> LayoutStyle {
    LayoutStyle {
        width: Some(LengthSpec::Px(width)),
        height: Some(LengthSpec::Px(height)),
        flex_shrink: Some(0.0),
        ..LayoutStyle::default()
    }
}

fn column(position: PositionSpec, width: f32) -> LayoutStyle {
    LayoutStyle {
        position,
        width: Some(LengthSpec::Px(width)),
        direction: Some(FlexDirection::Column),
        flex_shrink: Some(0.0),
        ..LayoutStyle::default()
    }
}

fn scroller(width: f32, height: f32, overflow: OverflowSpec) -> LayoutStyle {
    LayoutStyle {
        overflow_x: overflow,
        overflow_y: overflow,
        direction: Some(FlexDirection::Column),
        ..sized(width, height)
    }
}

fn placed(position: PositionSpec, [left, top, width, height]: [f32; 4]) -> LayoutStyle {
    LayoutStyle {
        position,
        offset_left: Some(LengthSpec::Px(left)),
        offset_top: Some(LengthSpec::Px(top)),
        background: Some([0.2, 0.4, 0.8, 1.0]),
        ..sized(width, height)
    }
}

struct Fixture {
    runtime: RuntimeDocument,
    viewport: LayoutViewport,
    outer: StableNodeId,
    card: StableNodeId,
    inner: StableNodeId,
    /// Each target and where its layout box is.
    targets: [(StableNodeId, LayoutBox); 4],
}

fn fixture() -> Fixture {
    let document = DocumentId::new(1).unwrap();
    let mut next = 0;
    let mut queue = MutationQueue::new();
    let mut element = |queue: &mut MutationQueue, parent: Option<StableNodeId>, layout| {
        next += 1;
        let id = StableNodeId::new(next).unwrap();
        match parent {
            None => queue.create(id, document, NodeKind::Document),
            Some(parent) => {
                queue.create(id, document, NodeKind::Element { tag: "div".into() });
                queue.insert(parent, id, None);
                queue.set_style(
                    id,
                    NodeStyle {
                        layout: Arc::new(layout),
                        ..NodeStyle::default()
                    },
                );
            }
        }
        id
    };
    let root = element(&mut queue, None, LayoutStyle::default());
    let page = element(
        &mut queue,
        Some(root),
        column(PositionSpec::Relative, 400.0),
    );
    let outer = element(
        &mut queue,
        Some(page),
        scroller(300.0, 200.0, OverflowSpec::Auto),
    );
    let card = element(
        &mut queue,
        Some(outer),
        column(PositionSpec::Relative, 300.0),
    );
    element(&mut queue, Some(card), sized(300.0, 50.0));
    let inner = element(
        &mut queue,
        Some(card),
        scroller(200.0, 100.0, OverflowSpec::Auto),
    );
    let row = element(
        &mut queue,
        Some(inner),
        LayoutStyle {
            background: Some([0.8, 0.2, 0.2, 1.0]),
            ..sized(100.0, 30.0)
        },
    );
    element(&mut queue, Some(inner), sized(200.0, 400.0));
    let escaping = element(
        &mut queue,
        Some(inner),
        placed(PositionSpec::Absolute, [150.0, 60.0, 80.0, 30.0]),
    );
    let fixed = element(
        &mut queue,
        Some(inner),
        placed(PositionSpec::Fixed, [320.0, 10.0, 30.0, 30.0]),
    );
    let side = element(
        &mut queue,
        Some(outer),
        scroller(300.0, 60.0, OverflowSpec::Hidden),
    );
    let escaping_far = element(
        &mut queue,
        Some(side),
        placed(PositionSpec::Absolute, [250.0, 150.0, 100.0, 40.0]),
    );
    element(&mut queue, Some(outer), sized(300.0, 500.0));
    for id in [row, escaping, fixed, escaping_far] {
        queue.set_interaction(
            id,
            InteractionState {
                pointer_events: true,
                focusable: false,
            },
        );
    }
    let mut runtime = RuntimeDocument::new(document);
    runtime.context_mut().commit_mutations(queue).unwrap();
    let viewport = LayoutViewport::new(800.0, 600.0);
    runtime.flush(viewport, &mut NoText).unwrap();
    let rect = |x, y, width, height| LayoutBox {
        x,
        y,
        width,
        height,
    };
    Fixture {
        runtime,
        viewport,
        outer,
        card,
        inner,
        targets: [
            (row, rect(0.0, 50.0, 100.0, 30.0)),
            (escaping, rect(150.0, 60.0, 80.0, 30.0)),
            (fixed, rect(320.0, 10.0, 30.0, 30.0)),
            (escaping_far, rect(250.0, 150.0, 100.0, 40.0)),
        ],
    }
}

/// The axis-aligned box `rect` covers under `transform`.
fn mapped(rect: SceneRect, transform: AffineTransform) -> LayoutBox {
    let [a, b, c, d, e, f] = transform.0;
    let corners = [
        (rect.x, rect.y),
        (rect.x + rect.width, rect.y),
        (rect.x, rect.y + rect.height),
        (rect.x + rect.width, rect.y + rect.height),
    ]
    .map(|(x, y)| (a * x + c * y + e, b * x + d * y + f));
    let left = corners.iter().map(|p| p.0).fold(f32::INFINITY, f32::min);
    let top = corners.iter().map(|p| p.1).fold(f32::INFINITY, f32::min);
    let right = corners
        .iter()
        .map(|p| p.0)
        .fold(f32::NEG_INFINITY, f32::max);
    let bottom = corners
        .iter()
        .map(|p| p.1)
        .fold(f32::NEG_INFINITY, f32::max);
    LayoutBox {
        x: left,
        y: top,
        width: right - left,
        height: bottom - top,
    }
}

fn intersect(a: LayoutBox, b: LayoutBox) -> LayoutBox {
    let left = a.x.max(b.x);
    let top = a.y.max(b.y);
    LayoutBox {
        x: left,
        y: top,
        width: ((a.x + a.width).min(b.x + b.width) - left).max(0.0),
        height: ((a.y + a.height).min(b.y + b.height) - top).max(0.0),
    }
}

fn close(a: LayoutBox, b: LayoutBox) -> bool {
    (a.x - b.x).abs() < 0.01
        && (a.y - b.y).abs() < 0.01
        && (a.width - b.width).abs() < 0.01
        && (a.height - b.height).abs() < 0.01
}

/// Paint, accessibility and the pointer agree on where each target is and
/// what is left of it.
fn assert_agree(fixture: &Fixture, at: &str) {
    let scene = fixture.runtime.scene();
    let world = fixture.runtime.context().world();
    let document = DocumentId::new(1).unwrap();
    let accessible = world.project_accessibility(document);
    for (id, layout) in fixture.targets {
        let primitive = scene
            .primitives()
            .find(|primitive| primitive.node == id)
            .unwrap_or_else(|| panic!("{at}: {id:?} paints"));
        let draw = scene.draw_primitive(primitive.id).unwrap();
        let painted = mapped(
            SceneRect {
                x: layout.x,
                y: layout.y,
                width: layout.width,
                height: layout.height,
            },
            draw.transform,
        );
        let painted_clip = draw
            .clips
            .iter()
            .map(|clip| mapped(clip.bounds, clip.transform))
            .reduce(intersect);
        let node = accessible
            .iter()
            .find(|node| node.id == id)
            .unwrap_or_else(|| panic!("{at}: {id:?} is accessible"));
        assert!(
            close(painted, node.bounds),
            "{at}: {id:?} paints at {painted:?}, reads at {:?}",
            node.bounds
        );
        match (painted_clip, node.clip) {
            (None, None) => {}
            (Some(painted), Some(read)) if close(painted, read) => {}
            (painted, read) => panic!("{at}: {id:?} is cut to {painted:?} in paint, {read:?} read"),
        }
        // The pointer finds the box in the middle of what is shown of it,
        // and not just outside that.
        let shown = node
            .clip
            .map_or(node.bounds, |clip| intersect(node.bounds, clip));
        // Culling keeps it where it shows, scrolls it has escaped
        // notwithstanding.
        if shown.width > 0.0 && shown.height > 0.0 {
            let visible = scene
                .visible_operations(SceneRect {
                    x: shown.x,
                    y: shown.y,
                    width: shown.width,
                    height: shown.height,
                })
                .unwrap();
            assert!(
                visible
                    .iter()
                    .any(|op| matches!(op, RenderOperation::Draw(drawn) if *drawn == primitive.id)),
                "{at}: {id:?} is culled where it shows"
            );
        }
        if shown.width > 2.0 && shown.height > 2.0 {
            let (x, y) = (shown.x + shown.width / 2.0, shown.y + shown.height / 2.0);
            assert_eq!(
                world.hit_test(document, x, y),
                Some(id),
                "{at}: {id:?} at {x},{y}"
            );
        }
        if shown.height < node.bounds.height - 4.0 {
            let x = shown.x + shown.width / 2.0;
            let y = if shown.y > node.bounds.y + 2.0 {
                shown.y - 2.0
            } else {
                shown.y + shown.height + 2.0
            };
            assert_ne!(
                world.hit_test(document, x, y),
                Some(id),
                "{at}: {id:?} cut at {x},{y}"
            );
        }
    }
}

#[test]
fn issue201_paint_hit_and_accessibility_share_one_clip_chain() {
    let mut fixture = fixture();
    let generation = fixture.runtime.context().world().layout_generation();
    assert_agree(&fixture, "at rest");
    // The boxes the chain lets escape are not cut by what they escape.
    let accessible = fixture
        .runtime
        .context()
        .world()
        .project_accessibility(DocumentId::new(1).unwrap());
    let clip_of = |id| accessible.iter().find(|node| node.id == id).unwrap().clip;
    let [row, escaping, fixed, far] = fixture.targets.map(|(id, _)| id);
    assert_eq!(
        clip_of(row).map(|clip| clip.width),
        Some(200.0),
        "the row is cut by the inner scroller"
    );
    assert_eq!(
        clip_of(escaping).map(|clip| clip.width),
        Some(300.0),
        "only the outer scroller cuts the card's absolute box"
    );
    assert_eq!((clip_of(fixed), clip_of(far)), (None, None));
    for (inner, outer) in [
        (40.0, 0.0),
        (40.0, 30.0),
        (0.0, 30.0),
        (60.0, 70.0),
        (0.0, 0.0),
    ] {
        let mut queue = MutationQueue::new();
        queue.set_scroll_offset(fixture.inner, ScrollOffset { x: 0.0, y: inner });
        queue.set_scroll_offset(fixture.outer, ScrollOffset { x: 0.0, y: outer });
        fixture
            .runtime
            .context_mut()
            .commit_mutations(queue)
            .unwrap();
        let update = fixture
            .runtime
            .flush(fixture.viewport, &mut NoText)
            .unwrap();
        assert_eq!(
            update.layout_generation, generation,
            "scrolling lays nothing out"
        );
        assert_agree(&fixture, &format!("inner {inner}, outer {outer}"));
    }
}

/// The chain moves under a scrolled page: the card stops being the block
/// (its absolute box escapes the outer scroller too, and stops scrolling
/// with it), the inner scroller stops clipping, and back. Paint, the
/// retained hit index and accessibility follow every step.
#[test]
fn issue201_a_moving_clip_chain_reaches_paint_hit_and_accessibility() {
    let mut fixture = fixture();
    let mut queue = MutationQueue::new();
    queue.set_scroll_offset(fixture.inner, ScrollOffset { x: 0.0, y: 20.0 });
    queue.set_scroll_offset(fixture.outer, ScrollOffset { x: 0.0, y: 30.0 });
    fixture
        .runtime
        .context_mut()
        .commit_mutations(queue)
        .unwrap();
    fixture
        .runtime
        .flush(fixture.viewport, &mut NoText)
        .unwrap();
    assert_agree(&fixture, "scrolled");
    for (position, overflow) in [
        (PositionSpec::Static, OverflowSpec::Auto),
        (PositionSpec::Static, OverflowSpec::Visible),
        (PositionSpec::Relative, OverflowSpec::Visible),
        (PositionSpec::Relative, OverflowSpec::Auto),
    ] {
        let mut queue = MutationQueue::new();
        queue.set_style(
            fixture.card,
            NodeStyle {
                layout: Arc::new(column(position, 300.0)),
                ..NodeStyle::default()
            },
        );
        queue.set_style(
            fixture.inner,
            NodeStyle {
                layout: Arc::new(scroller(200.0, 100.0, overflow)),
                ..NodeStyle::default()
            },
        );
        fixture
            .runtime
            .context_mut()
            .commit_mutations(queue)
            .unwrap();
        fixture
            .runtime
            .flush(fixture.viewport, &mut NoText)
            .unwrap();
        // Where the card's absolute box hangs from now.
        let escaping = fixture.targets[1].0;
        let layout = fixture
            .runtime
            .context()
            .world()
            .layout_box(escaping)
            .unwrap();
        fixture.targets[1].1 = layout;
        let at = format!("{position:?} card, {overflow:?} inner");
        assert_agree(&fixture, &at);
        // Then scrolled further.
        let mut queue = MutationQueue::new();
        queue.set_scroll_offset(fixture.outer, ScrollOffset { x: 0.0, y: 45.0 });
        fixture
            .runtime
            .context_mut()
            .commit_mutations(queue)
            .unwrap();
        fixture
            .runtime
            .flush(fixture.viewport, &mut NoText)
            .unwrap();
        assert_agree(&fixture, &format!("{at}, outer 45"));
        let mut queue = MutationQueue::new();
        queue.set_scroll_offset(fixture.outer, ScrollOffset { x: 0.0, y: 30.0 });
        fixture
            .runtime
            .context_mut()
            .commit_mutations(queue)
            .unwrap();
        fixture
            .runtime
            .flush(fixture.viewport, &mut NoText)
            .unwrap();
    }
}
