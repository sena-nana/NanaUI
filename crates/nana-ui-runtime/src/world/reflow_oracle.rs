//! The cold reflow oracle the incremental-reflow gates share (Issue #259),
//! and the plumbing their fixtures share: ids and styles, a builder that
//! hands out ids in order, the product frame, and what the last layout pass
//! reached.
//!
//! An incremental frame is correct when the same canonical inputs laid out
//! from nothing give the same geometry. [`assert_matches_cold`] compares two
//! contexts built from the same inputs, one driven through incremental
//! frames and one laid out once: every box, every published result (bounds,
//! baselines, fragments, overflow, scroll extent, containing block and the
//! rest of [`crate::LayoutResult::geometry_eq`]), every hit entry and every
//! accessibility bound.
//!
//! The per-pass guard in `layout_engine::verify` checks boxes after every
//! retained pass; this oracle checks what a pass publishes, against a world
//! that never ran an incremental frame, so state a pass keeps between frames
//! (shaped text, plans, retained sizes) cannot hide in both sides.

#![cfg(test)]

use std::collections::HashSet;
use std::sync::{Arc, Mutex};

use nana_text::NativeTextEngine;
use nana_text::font::{FaceDescriptor, FallbackPolicy, FontSystem, GenericFamily};
use nana_ui_core::{FlexDirection, InvalidationKind, LayoutStyle, LengthSpec, WorkCounters};

use super::{DocumentId, NodeKind, StableNodeId, UiWorld};
use crate::{
    AppContext, LayoutViewport, MutationQueue, NanaTextEngineShaper, NodeStyle, TextContent,
};

pub(super) fn node(value: u64) -> StableNodeId {
    StableNodeId::new(value).unwrap()
}

pub(super) fn styled(layout: LayoutStyle) -> NodeStyle {
    NodeStyle {
        layout: Arc::new(layout),
        ..NodeStyle::default()
    }
}

/// A column `width` wide, or as wide as what it holds.
pub(super) fn column(width: Option<f32>) -> LayoutStyle {
    LayoutStyle {
        width: width.map(LengthSpec::Px),
        direction: Some(FlexDirection::Column),
        ..LayoutStyle::default()
    }
}

/// A fixed `width` x `height` row.
pub(super) fn fixed(width: f32, height: f32) -> LayoutStyle {
    LayoutStyle {
        width: Some(LengthSpec::Px(width)),
        height: Some(LengthSpec::Px(height)),
        direction: Some(FlexDirection::Row),
        ..LayoutStyle::default()
    }
}

/// Cards in a group of [`Builder::filler`].
pub(super) const FILLER_CARDS: u64 = 10;
/// Nodes in a group of [`Builder::filler`]: the group, each card and its
/// label.
pub(super) const FILLER_GROUP_NODES: u64 = 1 + 2 * FILLER_CARDS;

/// One document's nodes, ids handed out in order.
pub(super) struct Builder {
    pub(super) queue: MutationQueue,
    document: DocumentId,
    next: u64,
}

impl Builder {
    /// A document node at id `first`; the nodes after it take the ids that
    /// follow.
    pub(super) fn new(document: DocumentId, first: u64) -> (Self, StableNodeId) {
        let mut builder = Self {
            queue: MutationQueue::new(),
            document,
            next: first,
        };
        let root = builder.take();
        builder.queue.create(root, document, NodeKind::Document);
        (builder, root)
    }

    /// [`Self::new`], and a page column `width` wide under the document.
    pub(super) fn page(document: DocumentId, first: u64, width: f32) -> (Self, StableNodeId) {
        let (mut builder, root) = Self::new(document, first);
        let page = builder.element(root, column(Some(width)));
        (builder, page)
    }

    fn take(&mut self) -> StableNodeId {
        self.next += 1;
        node(self.next - 1)
    }

    /// An element with no parent yet.
    pub(super) fn detached(&mut self, layout: LayoutStyle) -> StableNodeId {
        let id = self.take();
        self.queue
            .create(id, self.document, NodeKind::Element { tag: "div".into() });
        self.queue.set_style(id, styled(layout));
        id
    }

    pub(super) fn element(&mut self, parent: StableNodeId, layout: LayoutStyle) -> StableNodeId {
        let id = self.take();
        self.queue
            .create(id, self.document, NodeKind::Element { tag: "div".into() });
        self.queue.insert(parent, id, None);
        self.queue.set_style(id, styled(layout));
        id
    }

    pub(super) fn label(&mut self, parent: StableNodeId, value: &str) -> StableNodeId {
        let id = self.take();
        self.queue.create(id, self.document, NodeKind::Text);
        self.queue.insert(parent, id, None);
        self.queue.set_text(
            id,
            TextContent {
                value: value.into(),
            },
        );
        id
    }

    /// `groups` groups of [`FILLER_CARDS`] fixed 200x40 cards, each with the
    /// label "one".
    pub(super) fn filler(&mut self, parent: StableNodeId, groups: u64) {
        for _ in 0..groups {
            let group = self.element(parent, column(None));
            for _ in 0..FILLER_CARDS {
                let card = self.element(group, fixed(200.0, 40.0));
                self.label(card, "one");
            }
        }
    }
}

/// The product text engine over the bundled UI face alone, so a label
/// measures the same on every machine.
pub(super) fn bundled_face_shaper() -> NanaTextEngineShaper {
    let mut policy = FallbackPolicy::empty();
    policy.set_generic(GenericFamily::SansSerif, ["Noto Sans SC"]);
    let mut fonts = FontSystem::with_policy(policy);
    fonts
        .register_file(
            concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/../nana-ui/assets/fonts/NotoSansSC-Regular.ttf"
            ),
            &FaceDescriptor::default(),
        )
        .expect("the bundled UI face registers");
    NanaTextEngineShaper::new(Arc::new(Mutex::new(NativeTextEngine::new(fonts))))
}

/// One product frame, in the order `RuntimeDocument::flush` runs it: shape
/// scheduled text, merge what shaping queued, lay out, re-shape the laid-out
/// scope. Returns the frame's work counters.
pub(super) fn product_frame(
    context: &mut AppContext,
    document: DocumentId,
    viewport: LayoutViewport,
    shaper: &mut NanaTextEngineShaper,
) -> WorkCounters {
    frame(context, document, viewport, shaper, false, None)
}

/// [`product_frame`], and the nodes every layout pass of it was seeded at.
pub(super) fn product_frame_seeded(
    context: &mut AppContext,
    document: DocumentId,
    viewport: LayoutViewport,
    shaper: &mut NanaTextEngineShaper,
) -> (WorkCounters, HashSet<StableNodeId>) {
    let mut seeded = HashSet::new();
    let counters = frame(
        context,
        document,
        viewport,
        shaper,
        false,
        Some(&mut seeded),
    );
    (counters, seeded)
}

/// A product frame whose viewport changed: its first layout is the viewport
/// relayout, with the seeds the frame drained, as `RuntimeDocument::flush`
/// runs it.
pub(super) fn resize_frame(
    context: &mut AppContext,
    document: DocumentId,
    viewport: LayoutViewport,
    shaper: &mut NanaTextEngineShaper,
) -> WorkCounters {
    frame(context, document, viewport, shaper, true, None)
}

fn frame(
    context: &mut AppContext,
    document: DocumentId,
    viewport: LayoutViewport,
    shaper: &mut NanaTextEngineShaper,
    mut resized: bool,
    mut seeded: Option<&mut HashSet<StableNodeId>>,
) -> WorkCounters {
    context.compat_world_mut().observe_text_shaper(shaper);
    context.begin_frame_profile();
    for _ in 0..8 {
        let work = context.take_system_work();
        if work.is_empty() && !resized {
            break;
        }
        context.resolve_styles(&work.style).unwrap();
        context.shape_text(&work.text, shaper).unwrap();
        let mut seeds = work.layout_frontier_seeds.clone();
        seeds.extend(context.take_layout_frontier_seeds(document));
        if let Some(seeded) = seeded.as_deref_mut() {
            seeded.extend(seeds.iter().map(|seed| seed.node));
        }
        if resized {
            context
                .layout_document_for_viewport(document, viewport, &seeds)
                .unwrap();
            resized = false;
        } else if seeds.is_empty() {
            continue;
        } else {
            context
                .layout_document_with_frontier(document, viewport, &seeds)
                .unwrap();
        }
        let mut scope = context.take_last_layout_scope();
        for _ in 0..4 {
            if !context
                .shape_text_for_layout_scoped(&scope, shaper)
                .unwrap()
            {
                break;
            }
            let seeds = context.take_layout_frontier_seeds(document);
            if seeds.is_empty() {
                break;
            }
            if let Some(seeded) = seeded.as_deref_mut() {
                seeded.extend(seeds.iter().map(|seed| seed.node));
            }
            context
                .layout_document_with_frontier(document, viewport, &seeds)
                .unwrap();
            scope = context.take_last_layout_scope();
        }
    }
    context.finish_frame_profile();
    context.last_work_counters()
}

/// Whether the last layout pass of `id`'s document admitted it.
pub(super) fn admitted(context: &AppContext, id: StableNodeId) -> bool {
    context.layout_cause(id).is_some_and(|cause| !cause.pending)
}

/// Whether the last layout pass of `id`'s document measured it.
pub(super) fn measured(context: &AppContext, id: StableNodeId) -> bool {
    context
        .layout_cause(id)
        .filter(|cause| !cause.pending)
        .is_some_and(|cause| {
            cause
                .invalidation
                .kind
                .intersects(InvalidationKind::MEASURE)
        })
}

/// Nodes the last layout pass of `document` admitted to its frontier.
pub(super) fn admitted_nodes(context: &AppContext, document: DocumentId) -> HashSet<StableNodeId> {
    context
        .world()
        .document_order(document)
        .into_iter()
        .filter(|id| admitted(context, *id))
        .collect()
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct HitGeometry {
    layout: crate::LayoutBox,
    transform: [f32; 6],
}

fn hit_geometry(world: &UiWorld, document: DocumentId, id: StableNodeId) -> Option<HitGeometry> {
    let index = world.hit_test_index.get(&document)?;
    let hit = index.entries.get(&id)?;
    Some(HitGeometry {
        layout: hit.entry.layout,
        transform: super::hit_test::find_hit_transform(index, id)?,
    })
}

fn accessibility_bounds(world: &UiWorld, id: StableNodeId) -> Option<crate::LayoutBox> {
    world
        .project_accessibility_nodes(&[id])
        .into_iter()
        .find(|node| node.id == id)
        .map(|node| node.bounds)
}

/// Every box, published result, hit entry and accessibility bound of
/// `incremental` equals `cold`, the same inputs laid out once.
pub(super) fn assert_matches_cold(
    incremental: &mut AppContext,
    cold: &mut AppContext,
    document: DocumentId,
) {
    incremental.compat_world_mut().rebuild_hit_test(document);
    cold.compat_world_mut().rebuild_hit_test(document);
    let ours = incremental.world();
    let theirs = cold.world();
    let order = ours.document_order(document);
    assert_eq!(order, theirs.document_order(document));
    for id in order {
        assert_eq!(ours.layout_box(id), theirs.layout_box(id), "box of {id:?}");
        match (ours.layout_result(id), theirs.layout_result(id)) {
            (Some(ours), Some(theirs)) => assert!(
                ours.geometry_eq(theirs),
                "result of {id:?}: {ours:?} != {theirs:?}"
            ),
            (None, None) => {}
            (ours, theirs) => panic!("result of {id:?}: {ours:?} != {theirs:?}"),
        }
        assert_eq!(
            hit_geometry(ours, document, id),
            hit_geometry(theirs, document, id),
            "hit geometry of {id:?}"
        );
        assert_eq!(
            accessibility_bounds(ours, id),
            accessibility_bounds(theirs, id),
            "accessibility bounds of {id:?}"
        );
    }
}
