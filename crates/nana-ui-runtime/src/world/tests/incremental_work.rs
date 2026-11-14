//! Incremental invalidation, dirty frontier, axis constraints, virtual scroll deltas, resize propagation, and scale gates.

#![cfg(test)]

mod invalidation_classification {
    //! Issue #255: the layout invalidation contract.
    //!
    //! A style write is classified once, against that node's own fields. A write
    //! equal in effect queues nothing and counts as skipped; a real change queues
    //! one typed cause that says why (`reason`, `changed_inputs`) and what may
    //! follow from it (`kind`, `affected_axes`). Classification does not walk the
    //! document or the node's descendants (Gate A), keeps one fixed slot per node
    //! (Gate B), leaves paint, transform and accessibility writes out of layout
    //! (Gate C) and skips repeated equivalent writes (Gate D). DevTools reads the
    //! queued cause, and after a pass the cause the frontier admitted each node
    //! with.

    use std::sync::Arc;

    use nana_ui_core::{
        BoxShadowSpec, InvalidationKind, InvalidationReason, LayoutDependencyFootprint,
        LayoutFieldMask, LayoutInvalidation, LayoutStyle, LengthSpec, PaintTransform,
        SemanticColorRole, VisibilitySpec, WorkCounters,
    };

    use crate::world::reflow_oracle::{column, node, styled};
    use crate::world::{DocumentId, NodeKind, StableNodeId, UiWorld};
    use crate::{
        AccessibilityRole, AccessibilityState, AppContext, LayoutViewport, MutationQueue, NodeStyle,
    };

    fn fixed(width: f32, height: f32) -> NodeStyle {
        styled(LayoutStyle {
            width: Some(LengthSpec::Px(width)),
            height: Some(LengthSpec::Px(height)),
            ..LayoutStyle::default()
        })
    }

    /// A 240px column holding `rows` fixed 200x20 rows. Node 2 is the column,
    /// nodes 3.. are the rows.
    fn install_column(world: &mut UiWorld, rows: u64) -> DocumentId {
        let document = DocumentId::new(1).unwrap();
        let mut queue = MutationQueue::new();
        queue.create(node(1), document, NodeKind::Document);
        queue.create(node(2), document, NodeKind::Element { tag: "div".into() });
        queue.insert(node(1), node(2), None);
        queue.set_style(node(2), styled(column(Some(240.0))));
        for index in 0..rows {
            let row = node(3 + index);
            queue.create(row, document, NodeKind::Element { tag: "div".into() });
            queue.insert(node(2), row, None);
            queue.set_style(row, fixed(200.0, 20.0));
        }
        world.commit(queue).unwrap();
        document
    }

    /// Run frames until the world has nothing left to lay out.
    fn settle(context: &mut AppContext, document: DocumentId) {
        let viewport = LayoutViewport::new(320.0, 240.0);
        for _ in 0..8 {
            let work = context.take_system_work();
            if work.is_empty() {
                return;
            }
            context.resolve_styles(&work.style).unwrap();
            let mut seeds = work.layout_frontier_seeds.clone();
            seeds.extend(context.take_layout_frontier_seeds(document));
            if !seeds.is_empty() {
                context
                    .layout_document_with_frontier(document, viewport, &seeds)
                    .unwrap();
            }
        }
        panic!("frame did not settle");
    }

    fn commit_style(context: &mut AppContext, id: StableNodeId, edit: impl FnOnce(&mut NodeStyle)) {
        let mut next = context.world().node_style(id).unwrap().clone();
        edit(&mut next);
        let mut queue = MutationQueue::new();
        queue.set_style(id, next);
        context.compat_world_mut().commit(queue).unwrap();
    }

    fn rewrite_style(context: &mut AppContext, id: StableNodeId) {
        commit_style(context, id, |_| {});
    }

    /// Layout invalidations created, zero-delta writes and equivalent writes
    /// skipped between two counter snapshots.
    fn classified_counts(before: &WorkCounters, after: &WorkCounters) -> (usize, usize, usize) {
        (
            after.layout_invalidations_created - before.layout_invalidations_created,
            after.layout_invalidations_zero_delta - before.layout_invalidations_zero_delta,
            after.layout_equivalent_mutations_skipped - before.layout_equivalent_mutations_skipped,
        )
    }

    #[test]
    fn issue255_equivalent_write_is_skipped_and_a_width_change_queues_one_typed_cause() {
        let mut context = AppContext::new();
        let document = install_column(context.compat_world_mut(), 1);
        settle(&mut context, document);
        let row = node(3);

        let before = context.last_work_counters();
        rewrite_style(&mut context, row);
        // The same width, spelled as a fresh style object.
        commit_style(&mut context, row, |style| {
            Arc::make_mut(&mut style.layout).width = Some(LengthSpec::Px(200.0));
        });
        let after = context.last_work_counters();
        assert_eq!(classified_counts(&before, &after), (0, 2, 2));
        assert!(context.world().pending_layout_invalidation(row).is_empty());
        assert!(
            context.take_system_work().is_empty(),
            "an equivalent write schedules nothing"
        );

        let before = context.last_work_counters();
        commit_style(&mut context, row, |style| {
            Arc::make_mut(&mut style.layout).width = Some(LengthSpec::Px(180.0));
        });
        let after = context.last_work_counters();
        assert_eq!(classified_counts(&before, &after), (1, 0, 0));
        let cause = context.world().pending_layout_invalidation(row);
        assert!(
            cause.reason.contains(InvalidationReason::STYLE),
            "{cause:?}"
        );
        assert!(
            cause.changed_inputs.contains(LayoutFieldMask::SIZING),
            "{cause:?}"
        );
        assert!(cause.kind.contains(InvalidationKind::MEASURE), "{cause:?}");
        assert!(
            cause
                .affected_axes
                .contains(LayoutDependencyFootprint::EXPORTS_INTRINSIC_INLINE),
            "{cause:?}"
        );
    }

    /// DevTools explains a relayout: before the pass, the cause queued on the
    /// node; after it, the cause the frontier admitted each node with, and
    /// whether the node was the seed or a dependency edge reached it.
    #[test]
    fn issue255_devtools_explains_why_layout_reached_a_node() {
        let mut context = AppContext::new();
        let document = install_column(context.compat_world_mut(), 3);
        settle(&mut context, document);
        let row = node(3);
        let follower = node(4);

        commit_style(&mut context, row, |style| {
            Arc::make_mut(&mut style.layout).height = Some(LengthSpec::Px(30.0));
        });
        let queued = context.inspect(row).unwrap().layout.expect("queued cause");
        assert!(queued.pending && queued.seed);
        assert!(
            queued
                .invalidation
                .changed_inputs
                .contains(LayoutFieldMask::SIZING)
        );
        assert!(
            context
                .inspect(follower)
                .unwrap()
                .layout
                .is_none_or(|cause| !cause.pending),
            "nothing is queued on the follower itself"
        );

        settle(&mut context, document);
        let admitted = context.layout_cause(row).expect("the seed was laid out");
        assert!(!admitted.pending && admitted.seed);
        assert!(
            admitted
                .invalidation
                .kind
                .contains(InvalidationKind::MEASURE)
        );
        let reached = context
            .layout_cause(follower)
            .expect("the follower moved with the taller row");
        assert!(!reached.pending && !reached.seed);
        assert!(
            reached
                .invalidation
                .kind
                .contains(InvalidationKind::PLACEMENT),
            "{reached:?}"
        );
        assert!(
            !reached
                .invalidation
                .kind
                .contains(InvalidationKind::MEASURE),
            "the follower only moved: {reached:?}"
        );
    }

    /// What classifying one width change moves on the counters.
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    struct Classified {
        created: usize,
        zero_delta: usize,
        skipped: usize,
        footprints_read: usize,
        validation_nodes_scanned: Option<usize>,
    }

    fn classify_one_width_change(rows: u64) -> (Classified, LayoutInvalidation) {
        let mut context = AppContext::new();
        install_column(context.compat_world_mut(), rows);
        let column = node(2);
        assert_eq!(
            context.world().node(column).unwrap().children.len(),
            usize::try_from(rows).unwrap()
        );
        // Drain the install without laying out: only the write below is counted.
        let _ = context.take_system_work();
        let before = context.last_work_counters();
        commit_style(&mut context, column, |style| {
            Arc::make_mut(&mut style.layout).width = Some(LengthSpec::Px(180.0));
        });
        let after = context.last_work_counters();
        let work = context.take_system_work();
        let (created, zero_delta, skipped) = classified_counts(&before, &after);
        (
            Classified {
                created,
                zero_delta,
                skipped,
                footprints_read: after.layout_dependency_edges_visited
                    - before.layout_dependency_edges_visited,
                validation_nodes_scanned: work.validation_nodes_scanned,
            },
            work.layout_frontier_seeds
                .iter()
                .find(|seed| seed.node == column)
                .map(|seed| seed.invalidation)
                .unwrap_or_else(LayoutInvalidation::none),
        )
    }

    /// Gate A. The same width change on a column with 10k and with 100k rows is
    /// classified with the same counts and the same typed cause, and reads no
    /// dependency footprint: classification is the field diff of that one node.
    #[test]
    fn issue255_classifying_a_width_change_does_not_scale_with_the_document() {
        let (small, small_cause) = classify_one_width_change(10_000);
        let (large, large_cause) = classify_one_width_change(100_000);
        assert_eq!(small, large);
        assert_eq!(
            (
                small.created,
                small.zero_delta,
                small.skipped,
                small.footprints_read
            ),
            (1, 0, 0, 0)
        );
        assert_eq!(small_cause, large_cause);
        assert!(small_cause.changed_inputs.contains(LayoutFieldMask::SIZING));
    }

    /// Gate A, on published results. A layout field of the column does not hide
    /// the results of its rows until the next pass: the write does not walk them.
    #[test]
    fn issue255_a_layout_write_leaves_descendant_results_published() {
        let mut context = AppContext::new();
        let document = install_column(context.compat_world_mut(), 2_000);
        settle(&mut context, document);
        let column = node(2);
        let last_row = node(3 + 1_999);
        assert!(context.world().layout_result(last_row).is_some());
        commit_style(&mut context, column, |style| {
            Arc::make_mut(&mut style.layout).width = Some(LengthSpec::Px(220.0));
        });
        assert!(
            context.world().layout_result(column).is_none(),
            "the written node waits for its next result"
        );
        assert!(
            context.world().layout_result(last_row).is_some(),
            "a row result must not be hidden by a walk of the column's subtree"
        );
        settle(&mut context, document);
        assert!(context.world().layout_result(column).is_some());
        assert_eq!(context.world().layout_box(column).unwrap().width, 220.0);
    }

    fn assert_layout_uncharged(before: &WorkCounters, after: &WorkCounters, label: &str) {
        assert_eq!(
            after.layout_invalidations_created, before.layout_invalidations_created,
            "{label} must not create a layout invalidation"
        );
        assert_eq!(
            after.layout_dependency_edges_visited, before.layout_dependency_edges_visited,
            "{label} must not read a dependency footprint"
        );
        assert_eq!(
            after.layout_frontier_seeds, before.layout_frontier_seeds,
            "{label}"
        );
        assert_eq!(
            after.layout_measure_nodes, before.layout_measure_nodes,
            "{label}"
        );
        assert_eq!(
            after.layout_placement_nodes, before.layout_placement_nodes,
            "{label}"
        );
    }

    /// Gate C. Paint, colour, opacity, transform and accessibility writes on a
    /// laid-out node queue no layout cause and read no footprint, through a
    /// whole frame. A width change bundled with a colour still queues one.
    #[test]
    fn issue255_paint_opacity_transform_and_accessibility_stay_out_of_layout() {
        let mut context = AppContext::new();
        let document = install_column(context.compat_world_mut(), 1);
        settle(&mut context, document);
        let row = node(3);
        let laid_out = context.world().layout_box(row).unwrap();

        let writes: [(&str, Box<dyn Fn(&mut NodeStyle)>); 4] = [
            (
                "paint",
                Box::new(|style: &mut NodeStyle| {
                    let layout = Arc::make_mut(&mut style.layout);
                    layout.background = Some([0.1, 0.2, 0.3, 1.0]);
                    layout.border_color = Some([0.2, 0.2, 0.2, 1.0]);
                    layout.border_radius = Some(4.0);
                    layout.paint.visibility = Some(VisibilitySpec::Hidden);
                    layout.paint.box_shadows.push(BoxShadowSpec {
                        paint_color: None,
                        offset_x: 1.0,
                        offset_y: 1.0,
                        blur_radius: 2.0,
                        spread_radius: 0.0,
                        color: [0.0, 0.0, 0.0, 0.4],
                        inset: false,
                    });
                    style.foreground = Some(SemanticColorRole::Accent);
                    style.background = Some(SemanticColorRole::Surface);
                    style.border = Some(SemanticColorRole::Border);
                }),
            ),
            (
                "colour",
                Box::new(|style: &mut NodeStyle| {
                    Arc::make_mut(&mut style.layout).color = Some([0.2, 0.4, 0.8, 1.0]);
                }),
            ),
            (
                "opacity",
                Box::new(|style: &mut NodeStyle| {
                    Arc::make_mut(&mut style.layout).opacity = Some(0.4);
                }),
            ),
            (
                "transform",
                Box::new(|style: &mut NodeStyle| {
                    Arc::make_mut(&mut style.layout).transform = Some(PaintTransform {
                        e: 8.0,
                        ..PaintTransform::default()
                    });
                }),
            ),
        ];
        for (label, write) in writes {
            let before = context.last_work_counters();
            commit_style(&mut context, row, |style| write(style));
            assert!(
                context.world().pending_layout_invalidation(row).is_empty(),
                "{label}"
            );
            settle(&mut context, document);
            assert_layout_uncharged(&before, &context.last_work_counters(), label);
        }

        let before = context.last_work_counters();
        let mut queue = MutationQueue::new();
        queue.set_accessibility(
            row,
            AccessibilityState {
                role: AccessibilityRole::Button,
                label: Some(Arc::<str>::from("row")),
                disabled: true,
                ..AccessibilityState::default()
            },
        );
        context.compat_world_mut().commit(queue).unwrap();
        assert!(context.world().pending_layout_invalidation(row).is_empty());
        settle(&mut context, document);
        assert_layout_uncharged(&before, &context.last_work_counters(), "accessibility");
        assert_eq!(context.world().layout_box(row), Some(laid_out));

        let before = context.last_work_counters();
        commit_style(&mut context, row, |style| {
            let layout = Arc::make_mut(&mut style.layout);
            layout.width = Some(LengthSpec::Px(180.0));
            layout.color = Some([0.9, 0.1, 0.1, 1.0]);
        });
        let after = context.last_work_counters();
        assert_eq!(
            after.layout_invalidations_created,
            before.layout_invalidations_created + 1
        );
        assert_eq!(
            after.layout_dependency_edges_visited, before.layout_dependency_edges_visited,
            "classification reads no footprint; the frontier does, when layout runs"
        );
    }

    /// Gate D. One thousand effective-equivalent width writes on a 10k-row
    /// column are each skipped: nothing is queued, nothing is laid out, and the
    /// drain after them is empty.
    #[test]
    fn issue255_a_thousand_equivalent_writes_are_skipped_on_a_10k_tree() {
        let mut context = AppContext::new();
        install_column(context.compat_world_mut(), 10_000);
        let column = node(2);
        let _ = context.take_system_work();
        let before = context.last_work_counters();
        for _ in 0..1000 {
            commit_style(&mut context, column, |style| {
                Arc::make_mut(&mut style.layout).width = Some(LengthSpec::Px(240.0));
            });
        }
        let after = context.last_work_counters();
        assert_eq!(classified_counts(&before, &after), (0, 1000, 1000));
        assert_eq!(after.layout_frontier_seeds, before.layout_frontier_seeds);
        assert_eq!(
            after.layout_frontier_nodes_measure,
            before.layout_frontier_nodes_measure
        );
        assert_eq!(
            after.layout_frontier_nodes_placement,
            before.layout_frontier_nodes_placement
        );
        assert_eq!(context.layout_invocations(), 0);
        let idle = context.take_system_work();
        assert!(idle.is_empty());
        assert_eq!(idle.validation_nodes_scanned, Some(0));
    }

    /// Bytes the pending-cause table holds: its buckets, each one id and one
    /// [`LayoutInvalidation`] plus the control byte, whether full or not.
    fn pending_table_bytes(world: &UiWorld) -> usize {
        world.pending_layout_invalidations.capacity()
            * (std::mem::size_of::<(StableNodeId, LayoutInvalidation)>() + 1)
    }

    /// Gate B. Dependency metadata is one fixed cause per node, at most 64 bytes
    /// with the table that holds it, on a 100k-node tree where every node has a
    /// cause pending. Two thousand further writes to one node add nothing.
    #[test]
    #[ignore = "100k dense scale gate; run with --release -- --ignored"]
    fn issue255_dependency_metadata_stays_one_bounded_slot_per_node_at_100k() {
        assert!(
            std::mem::size_of::<LayoutInvalidation>() <= 16,
            "a typed cause is a few masks, not a node list"
        );
        assert_eq!(std::mem::size_of::<LayoutDependencyFootprint>(), 2);

        let mut context = AppContext::new();
        install_column(context.compat_world_mut(), 100_000);
        let column = node(2);
        let nodes = context.world().len();
        let slots = context.world().pending_layout_invalidations.len();
        assert!(slots <= nodes);
        let bytes = pending_table_bytes(context.world());
        assert!(
            bytes <= nodes * 64,
            "{bytes} bytes of pending causes for {nodes} nodes is above 64 bytes a node"
        );

        for width in [180.0, 160.0].into_iter().cycle().take(1000) {
            commit_style(&mut context, column, |style| {
                Arc::make_mut(&mut style.layout).width = Some(LengthSpec::Px(width));
            });
            rewrite_style(&mut context, column);
        }
        assert_eq!(
            context.world().pending_layout_invalidations.len(),
            slots,
            "repeat writes merge into the node's one slot"
        );
        assert_eq!(pending_table_bytes(context.world()), bytes);
        assert_eq!(context.layout_invocations(), 0);
    }
}

mod dirty_frontier {
    //! Issue #256: the dirty frontier.
    //!
    //! One fixture throughout: a page of groups, each group three wrappers deep,
    //! each wrapper holding ten fixed 200x40 cards with one label. A label is
    //! eight levels below the document. Frames run the order the product flush
    //! runs: shape scheduled text, merge what shaping queued, lay out, re-shape
    //! the laid-out scope.
    //!
    //! - Gate A/D: one label edit inside a fixed card costs the same at 1k, 10k
    //!   and 100k nodes, walks at most 32 dependency edges, and admits nothing
    //!   outside the card.
    //! - Gate B: text whose width, height and baseline held stops at the text:
    //!   no parent work, and the stop is counted.
    //! - Gate C: a hundred label edits in ten groups build one union closure and
    //!   walk each of its edges at most twice; the scratch they build is the
    //!   same at 1k and 10k nodes.
    //! - Correctness: after a run of edits, every box, published result, hit
    //!   entry and accessibility bound equals a cold layout of the same tree.

    use std::collections::HashSet;

    use nana_ui_core::WorkCounters;

    use crate::world::reflow_oracle::{
        admitted_nodes, assert_matches_cold, bundled_face_shaper, column, fixed, node,
        product_frame, styled,
    };
    use crate::world::{DocumentId, NodeKind, StableNodeId, UiWorld};
    use crate::{
        AppContext, LayoutFrontierStats, LayoutViewport, MutationQueue, NanaTextEngineShaper,
        TextContent,
    };

    fn viewport() -> LayoutViewport {
        LayoutViewport::new(320.0, 480.0)
    }

    /// One product frame, in the order `RuntimeDocument::flush` runs it.
    fn frame(
        context: &mut AppContext,
        document: DocumentId,
        shaper: &mut NanaTextEngineShaper,
    ) -> WorkCounters {
        product_frame(context, document, viewport(), shaper)
    }

    const WRAPPERS: u64 = 3;
    const CARDS: u64 = 10;
    const GROUP_NODES: u64 = 1 + WRAPPERS + 2 * CARDS;

    struct Cards {
        context: AppContext,
        document: DocumentId,
        shaper: NanaTextEngineShaper,
        groups: u64,
    }

    impl Cards {
        fn group(group: u64) -> StableNodeId {
            node(3 + group * GROUP_NODES)
        }

        fn wrapper(group: u64, depth: u64) -> StableNodeId {
            node(Self::group(group).get() + 1 + depth)
        }

        fn card(group: u64, card: u64) -> StableNodeId {
            node(Self::group(group).get() + 1 + WRAPPERS + 2 * card)
        }

        fn label(group: u64, card: u64) -> StableNodeId {
            node(Self::card(group, card).get() + 1)
        }

        /// About `nodes` nodes, every label reading `text(label)`.
        fn with_text(nodes: u64, text: impl Fn(StableNodeId) -> String) -> Self {
            let groups = (nodes.saturating_sub(2) / GROUP_NODES).max(1);
            let document = DocumentId::new(1).unwrap();
            let mut context = AppContext::new();
            let mut queue = MutationQueue::new();
            queue.create(node(1), document, NodeKind::Document);
            queue.create(node(2), document, NodeKind::Element { tag: "page".into() });
            queue.insert(node(1), node(2), None);
            queue.set_style(node(2), styled(column(Some(320.0))));
            for group in 0..groups {
                let mut parent = node(2);
                let group_id = Self::group(group);
                for (index, id) in std::iter::once(group_id)
                    .chain((0..WRAPPERS).map(|depth| Self::wrapper(group, depth)))
                    .enumerate()
                {
                    let tag = if index == 0 { "group" } else { "wrapper" };
                    queue.create(id, document, NodeKind::Element { tag: tag.into() });
                    queue.insert(parent, id, None);
                    queue.set_style(id, styled(column(None)));
                    parent = id;
                }
                for card in 0..CARDS {
                    let card_id = Self::card(group, card);
                    let label_id = Self::label(group, card);
                    queue.create(card_id, document, NodeKind::Element { tag: "card".into() });
                    queue.insert(parent, card_id, None);
                    queue.set_style(card_id, styled(fixed(200.0, 40.0)));
                    queue.create(label_id, document, NodeKind::Text);
                    queue.insert(card_id, label_id, None);
                    queue.set_text(
                        label_id,
                        TextContent {
                            value: text(label_id).into(),
                        },
                    );
                }
            }
            context.compat_world_mut().commit(queue).unwrap();
            let mut shaper = bundled_face_shaper();
            frame(&mut context, document, &mut shaper);
            Self {
                context,
                document,
                shaper,
                groups,
            }
        }

        fn new(nodes: u64) -> Self {
            Self::with_text(nodes, |_| "one".into())
        }

        fn set_text(&mut self, id: StableNodeId, value: &str) {
            let mut queue = MutationQueue::new();
            queue.set_text(
                id,
                TextContent {
                    value: value.into(),
                },
            );
            self.context.compat_world_mut().commit(queue).unwrap();
        }

        fn frame(&mut self) -> WorkCounters {
            frame(&mut self.context, self.document, &mut self.shaper)
        }

        fn frontier_stats(&self) -> LayoutFrontierStats {
            self.context.layout_frontier_stats(self.document)
        }

        fn world(&self) -> &UiWorld {
            self.context.world()
        }
    }

    /// What one frame cost the layout, in the units the gates name.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    struct Work {
        seeds: usize,
        measure: usize,
        placement: usize,
        contexts: usize,
        edges: usize,
        local_fallbacks: usize,
        full_fallbacks: usize,
        measured: usize,
        placed: usize,
    }

    impl From<WorkCounters> for Work {
        fn from(counters: WorkCounters) -> Self {
            Self {
                seeds: counters.layout_frontier_seeds,
                measure: counters.layout_frontier_nodes_measure,
                placement: counters.layout_frontier_nodes_placement,
                contexts: counters.layout_frontier_contexts,
                edges: counters.layout_dependency_edges_visited,
                local_fallbacks: counters.layout_local_subtree_fallbacks,
                full_fallbacks: counters.layout_full_document_fallbacks,
                measured: counters.layout_measure_nodes,
                placed: counters.layout_placement_nodes,
            }
        }
    }

    /// One label in the middle group grows; its fixed card absorbs it.
    fn contained_label_edit(nodes: u64) -> (Work, HashSet<StableNodeId>, Cards) {
        let mut cards = Cards::new(nodes);
        let group = cards.groups / 2;
        let label = Cards::label(group, 4);
        cards.set_text(label, "seventeen");
        let work = Work::from(cards.frame());
        let admitted = admitted_nodes(&cards.context, cards.document);
        (work, admitted, cards)
    }

    fn assert_contained(work: Work, admitted: &HashSet<StableNodeId>, cards: &Cards) {
        let group = cards.groups / 2;
        let label = Cards::label(group, 4);
        let card = Cards::card(group, 4);
        assert_eq!(cards.world().layout_box(card).unwrap().width, 200.0);
        assert!(work.edges <= 32, "{work:?}");
        assert_eq!((work.local_fallbacks, work.full_fallbacks), (0, 0));
        assert!(work.measure >= 1, "the label itself is measured: {work:?}");
        let path: HashSet<StableNodeId> = [label, card].into();
        assert!(
            admitted.is_subset(&path),
            "only the label and its fixed card enter the frontier: {admitted:?}"
        );
    }

    /// Gates A and D. The same contained edit at 1k and 10k nodes, depth eight.
    #[test]
    fn issue256_a_contained_label_edit_costs_the_same_at_1k_and_10k() {
        let (small, small_admitted, small_cards) = contained_label_edit(1_000);
        let (large, large_admitted, large_cards) = contained_label_edit(10_000);
        assert!(large_cards.world().len() >= 10 * small_cards.world().len() - 100);
        assert_contained(small, &small_admitted, &small_cards);
        assert_contained(large, &large_admitted, &large_cards);
        assert_eq!(small, large);
        assert_eq!(
            small_cards.frontier_stats().scratch_entries,
            large_cards.frontier_stats().scratch_entries,
            "the pass builds scratch for the closure, not the document"
        );
    }

    /// Gate D at 100k nodes.
    #[test]
    #[ignore = "100k dense scale gate; run with --release -- --ignored"]
    fn issue256_a_contained_label_edit_costs_the_same_at_100k() {
        let (small, ..) = contained_label_edit(1_000);
        let (large, large_admitted, large_cards) = contained_label_edit(100_000);
        assert!(large_cards.world().len() >= 99_000);
        assert_contained(large, &large_admitted, &large_cards);
        assert_eq!(small, large);
    }

    /// Gate B. "99" and "98" have the same width, height and baseline: the text
    /// is re-shaped, its parent is not measured, no dependency edge is walked,
    /// and the stop is counted.
    #[test]
    fn issue256_text_whose_exported_metrics_held_stops_at_the_text() {
        let mut cards = Cards::new(1_000);
        let group = cards.groups / 2;
        let label = Cards::label(group, 5);
        let card = Cards::card(group, 5);
        cards.set_text(label, "99");
        cards.frame();
        let label_box = cards.world().layout_box(label).unwrap();
        let card_box = cards.world().layout_box(card).unwrap();
        let baseline = cards
            .world()
            .layout_result(label)
            .and_then(|result| result.first_baseline);

        cards.set_text(label, "98");
        let counters = cards.frame();
        assert!(
            cards.world().last_text_work_counters().layouts_created > 0,
            "the text itself is laid out again"
        );
        assert_eq!(counters.layout_frontier_nodes_measure, 0);
        assert_eq!(counters.layout_dependency_edges_visited, 0);
        assert!(counters.layout_propagations_stopped >= 1);
        assert_eq!(cards.world().layout_box(label), Some(label_box));
        assert_eq!(cards.world().layout_box(card), Some(card_box));
        assert_eq!(
            cards
                .world()
                .layout_result(label)
                .and_then(|result| result.first_baseline),
            baseline
        );
    }

    /// The hundred labels of ten groups spread across the page.
    fn hundred_labels(groups: u64) -> Vec<StableNodeId> {
        (0..10)
            .map(|step| step * groups / 10)
            .flat_map(|group| (0..CARDS).map(move |card| Cards::label(group, card)))
            .collect()
    }

    fn hundred_seed_batch(nodes: u64) -> (WorkCounters, LayoutFrontierStats) {
        let mut cards = Cards::new(nodes);
        for label in hundred_labels(cards.groups) {
            cards.set_text(label, "seventeen");
        }
        let counters = cards.frame();
        (counters, cards.frontier_stats())
    }

    /// Gate C. A hundred edits in ten groups are one union closure: each edge of
    /// it is walked at most twice, not once per seed's walk to the root, and the
    /// scratch the pass builds does not grow with the document.
    #[test]
    fn issue256_a_hundred_seeds_walk_their_union_closure_once() {
        let (small, small_stats) = hundred_seed_batch(1_000);
        let (large, large_stats) = hundred_seed_batch(10_000);
        for (counters, stats) in [(small, small_stats), (large, large_stats)] {
            assert_eq!(counters.layout_frontier_seeds, stats.seeds);
            assert!(stats.seeds >= 100, "{stats:?}");
            assert!(
                counters.layout_dependency_edges_visited <= 2 * stats.graph_edges,
                "{} edge walks over a {}-edge closure",
                counters.layout_dependency_edges_visited,
                stats.graph_edges
            );
            assert_eq!(counters.layout_full_document_fallbacks, 0);
        }
        assert_eq!(Work::from(small), Work::from(large));
        assert_eq!(small_stats, large_stats);
    }

    /// Correctness. A grown label, an unchanged-metric edit, a hundred-seed
    /// batch and a batch whose text wraps, each in its own frame, leave the same
    /// geometry a cold layout of the final tree has.
    #[test]
    fn issue256_incremental_frames_match_a_cold_layout() {
        let mut cards = Cards::new(1_000);
        let groups = cards.groups;
        let mut final_text = std::collections::HashMap::new();
        let mut edit = |cards: &mut Cards, label: StableNodeId, text: &str| {
            cards.set_text(label, text);
            final_text.insert(label, text.to_owned());
        };

        edit(&mut cards, Cards::label(groups / 2, 4), "seventeen");
        cards.frame();
        edit(&mut cards, Cards::label(groups / 2, 5), "99");
        cards.frame();
        edit(&mut cards, Cards::label(groups / 2, 5), "98");
        cards.frame();
        for label in hundred_labels(groups) {
            edit(&mut cards, label, "seventeen");
        }
        cards.frame();
        for label in hundred_labels(groups).into_iter().step_by(3) {
            edit(
                &mut cards,
                label,
                "a label long enough to wrap inside its card",
            );
        }
        cards.frame();

        let mut cold = Cards::with_text(1_000, |label| {
            final_text
                .get(&label)
                .cloned()
                .unwrap_or_else(|| "one".into())
        });
        assert_matches_cold(&mut cards.context, &mut cold.context, cold.document);
    }

    /// Every edit of a child list keeps each child's recorded index in it, so
    /// the dependency graph finds a node's next sibling without scanning.
    #[test]
    fn issue256_child_indices_follow_inserts_moves_and_removals() {
        let document = DocumentId::new(1).unwrap();
        let mut context = AppContext::new();
        let mut queue = MutationQueue::new();
        queue.create(node(1), document, NodeKind::Document);
        for list in [2, 3] {
            queue.create(
                node(list),
                document,
                NodeKind::Element { tag: "list".into() },
            );
            queue.insert(node(1), node(list), None);
        }
        for row in 10..16 {
            queue.create(node(row), document, NodeKind::Element { tag: "row".into() });
            queue.insert(node(2), node(row), None);
        }
        context.compat_world_mut().commit(queue).unwrap();
        let indices_hold = |world: &UiWorld| {
            for list in [node(2), node(3)] {
                let children = world.nodes.get(list).unwrap().hierarchy.children.clone();
                for (index, child) in children.iter().enumerate() {
                    assert_eq!(
                        world.recorded_child_index(list, *child),
                        Some(index),
                        "{child:?} in {children:?}"
                    );
                }
            }
        };
        indices_hold(context.world());
        let edits: [&dyn Fn(&mut MutationQueue); 6] = [
            // A new row in the middle.
            &|queue| {
                queue.create(node(20), document, NodeKind::Element { tag: "row".into() });
                queue.insert(node(2), node(20), Some(node(12)));
            },
            // The last row to the front, then the first to the end.
            &|queue| queue.insert(node(2), node(15), Some(node(10))),
            &|queue| queue.insert(node(2), node(15), None),
            // A row into the other list.
            &|queue| queue.insert(node(3), node(11), None),
            &|queue| queue.detach(node(13)),
            &|queue| queue.despawn_subtree(node(20)),
        ];
        for edit in edits {
            let mut queue = MutationQueue::new();
            edit(&mut queue);
            context.compat_world_mut().commit(queue).unwrap();
            indices_hold(context.world());
        }
    }

    /// A child whose height goes from a length back to filling, under a parent
    /// whose height follows its content, lays out again: every pass matches a
    /// cold layout of the same inputs.
    #[test]
    fn issue256_a_height_back_to_fill_under_an_auto_parent_lays_out_again() {
        use nana_ui_core::{LayoutStyle, LengthSpec};
        let document = DocumentId::new(1).unwrap();
        let viewport = LayoutViewport::new(800.0, 600.0);
        let fill = LayoutStyle {
            width: Some(LengthSpec::Fill),
            height: Some(LengthSpec::Fill),
            ..LayoutStyle::default()
        };
        let build = |row_height: Option<LengthSpec>| {
            let mut queue = MutationQueue::new();
            queue.create(node(1), document, NodeKind::Document);
            let mut element = |id: u64, parent: u64, layout: LayoutStyle| {
                queue.create(node(id), document, NodeKind::Element { tag: "div".into() });
                queue.insert(node(parent), node(id), None);
                queue.set_style(node(id), styled(layout));
            };
            element(2, 1, column(Some(320.0)));
            element(3, 2, column(None));
            element(
                4,
                3,
                LayoutStyle {
                    height: row_height,
                    ..fill.clone()
                },
            );
            let mut context = AppContext::new();
            context.commit_mutations(queue).unwrap();
            context
        };
        let mut shaper = bundled_face_shaper();
        let mut context = build(Some(LengthSpec::Fill));
        product_frame(&mut context, document, viewport, &mut shaper);
        for height in [Some(LengthSpec::Px(40.0)), Some(LengthSpec::Fill)] {
            let mut queue = MutationQueue::new();
            queue.set_style(
                node(4),
                styled(LayoutStyle {
                    height,
                    ..fill.clone()
                }),
            );
            context.commit_mutations(queue).unwrap();
            product_frame(&mut context, document, viewport, &mut shaper);
        }
        let mut cold = build(Some(LengthSpec::Fill));
        product_frame(&mut cold, document, viewport, &mut shaper);
        assert_matches_cold(&mut context, &mut cold, document);
    }

    /// Labels wider than their rows: rows spanning a list that has no width of
    /// its own, so none, each hold words longer than that. A label is as wide as
    /// its widest word whichever box it was last shaped in -- none yet, the
    /// empty row, or the width that word gave it -- so every pass matches a full
    /// layout and the frames end as a cold layout of the same inputs does. Words
    /// that take a line each in the empty row and share one in the wider box
    /// settle there too.
    #[test]
    fn issue256_a_word_wider_than_its_row_keeps_its_width_after_a_reshape() {
        use nana_ui_core::{DisplaySpec, LayoutStyle, LengthSpec, PositionSpec};
        let document = DocumentId::new(1).unwrap();
        let viewport = LayoutViewport::new(320.0, 480.0);
        let label = |height: f32| {
            styled(LayoutStyle {
                height: Some(LengthSpec::Px(height)),
                ..LayoutStyle::default()
            })
        };
        for words in ["99", "9999 9 9"] {
            // Two rows of a list, each placed at its offset and holding a label.
            let rows = |queue: &mut MutationQueue, first_height: f32| {
                for (row, top, height) in [(4, 72.0, first_height), (6, 96.0, 24.0)] {
                    queue.create(node(row), document, NodeKind::Element { tag: "div".into() });
                    queue.insert(node(3), node(row), None);
                    queue.set_style(
                        node(row),
                        styled(LayoutStyle {
                            // A flex column keeps the label at its own width;
                            // a block row would stretch it to the row's.
                            display: Some(DisplaySpec::Flex),
                            position: PositionSpec::Absolute,
                            offset_top: Some(LengthSpec::Px(top)),
                            offset_left: Some(LengthSpec::Px(0.0)),
                            width: Some(LengthSpec::Percent(100.0)),
                            flex_shrink: Some(0.0),
                            ..LayoutStyle::default()
                        }),
                    );
                    queue.create(node(row + 1), document, NodeKind::Text);
                    queue.insert(node(row), node(row + 1), None);
                    queue.set_text(
                        node(row + 1),
                        TextContent {
                            value: words.into(),
                        },
                    );
                    queue.set_style(node(row + 1), label(height));
                }
            };
            let build = |first_height: Option<f32>| {
                let mut queue = MutationQueue::new();
                queue.create(node(1), document, NodeKind::Document);
                let mut element = |id: u64, parent: u64, layout: LayoutStyle| {
                    queue.create(node(id), document, NodeKind::Element { tag: "div".into() });
                    queue.insert(node(parent), node(id), None);
                    queue.set_style(node(id), styled(layout));
                };
                element(
                    2,
                    1,
                    LayoutStyle {
                        // A flex column keeps the list at its own (empty) width.
                        display: Some(DisplaySpec::Flex),
                        width: Some(LengthSpec::Px(320.0)),
                        ..LayoutStyle::default()
                    },
                );
                element(
                    3,
                    2,
                    LayoutStyle {
                        height: Some(LengthSpec::Px(200.0)),
                        ..LayoutStyle::default()
                    },
                );
                if let Some(height) = first_height {
                    rows(&mut queue, height);
                }
                let mut context = AppContext::new();
                context.commit_mutations(queue).unwrap();
                context
            };
            let mut shaper = bundled_face_shaper();
            let mut context = build(None);
            product_frame(&mut context, document, viewport, &mut shaper);
            let mut queue = MutationQueue::new();
            rows(&mut queue, 24.0);
            context.commit_mutations(queue).unwrap();
            product_frame(&mut context, document, viewport, &mut shaper);
            let mut queue = MutationQueue::new();
            queue.set_style(node(5), label(32.0));
            context.commit_mutations(queue).unwrap();
            product_frame(&mut context, document, viewport, &mut shaper);
            let world = context.world();
            for text in [node(5), node(7)] {
                assert_eq!(
                    world.layout_box(text).map(|layout| layout.width),
                    world.text_metrics(text).map(|metrics| metrics.width),
                    "{words:?}: a label is as wide as its widest line"
                );
            }
            let mut cold = build(Some(32.0));
            product_frame(&mut cold, document, viewport, &mut shaper);
            assert_matches_cold(&mut context, &mut cold, document);
        }
    }
}

mod axis_constraints {
    //! Issue 257: a parent constraint measures only children that consume that axis.

    use std::sync::{Arc, Mutex};

    use nana_text::NativeTextEngine;
    use nana_text::font::FontSystem;
    use nana_ui_core::{LayoutDependencyFootprint, LayoutStyle, LengthSpec};

    use crate::world::{DocumentId, NodeKind, StableNodeId, UiWorld};
    use crate::{
        LayoutFrontier, LayoutViewport, MutationQueue, NanaTextEngineShaper, NodeStyle,
        RetainedLayoutCache, RuntimeLayoutEngine, TextContent,
    };

    fn node(value: u64) -> StableNodeId {
        StableNodeId::new(value).unwrap()
    }

    fn document(value: u64) -> DocumentId {
        DocumentId::new(value).unwrap()
    }

    fn style(layout: LayoutStyle) -> NodeStyle {
        NodeStyle {
            layout: Arc::new(layout),
            ..NodeStyle::default()
        }
    }

    fn px_box(width: f32, height: f32) -> LayoutStyle {
        LayoutStyle {
            width: Some(LengthSpec::Px(width)),
            height: Some(LengthSpec::Px(height)),
            ..LayoutStyle::default()
        }
    }

    fn commit_style(world: &mut UiWorld, id: StableNodeId, layout: LayoutStyle) {
        let mut queue = MutationQueue::new();
        queue.set_style(id, style(layout));
        world.commit(queue).unwrap();
    }

    fn seeds_after(world: &mut UiWorld) -> Vec<crate::LayoutFrontierSeed> {
        world.take_system_work().layout_frontier_seeds
    }

    fn frontier_for(world: &UiWorld, seeds: &[crate::LayoutFrontierSeed]) -> LayoutFrontier {
        let graph = world.layout_dependency_graph_for_seeds(document(1), seeds);
        LayoutFrontier::from_dependency_graph(seeds.iter().copied(), &graph)
    }

    fn section_with_fixed_subtree(fixed_descendants: u64) -> (UiWorld, StableNodeId) {
        let mut world = UiWorld::new();
        let mut queue = MutationQueue::new();
        queue.create(node(1), document(1), NodeKind::Document);
        queue.create(
            node(2),
            document(1),
            NodeKind::Element {
                tag: "section".into(),
            },
        );
        queue.insert(node(1), node(2), None);
        queue.set_style(node(2), style(px_box(200.0, 120.0)));

        queue.create(
            node(5),
            document(1),
            NodeKind::Element {
                tag: "fixed".into(),
            },
        );
        queue.insert(node(2), node(5), None);
        queue.set_style(node(5), style(px_box(40.0, 30.0)));
        for offset in 0..fixed_descendants {
            let id = node(100 + offset);
            queue.create(
                id,
                document(1),
                NodeKind::Element {
                    tag: "fixed-child".into(),
                },
            );
            queue.insert(node(5), id, None);
            queue.set_style(id, style(px_box(10.0, 10.0)));
        }

        queue.create(
            node(3),
            document(1),
            NodeKind::Element {
                tag: "percent".into(),
            },
        );
        queue.insert(node(2), node(3), None);
        queue.set_style(
            node(3),
            style(LayoutStyle {
                width: Some(LengthSpec::Percent(50.0)),
                height: Some(LengthSpec::Px(20.0)),
                ..LayoutStyle::default()
            }),
        );
        queue.create(
            node(4),
            document(1),
            NodeKind::Element { tag: "fill".into() },
        );
        queue.insert(node(2), node(4), None);
        queue.set_style(
            node(4),
            style(LayoutStyle {
                width: Some(LengthSpec::Fill),
                height: Some(LengthSpec::Px(20.0)),
                ..LayoutStyle::default()
            }),
        );
        world.commit(queue).unwrap();
        let _ = world.take_system_work();
        (world, node(2))
    }

    fn fixed_subtree_measure_count(frontier: &LayoutFrontier, fixed_descendants: u64) -> usize {
        let measure = frontier.measure_nodes();
        let mut count = usize::from(measure.contains(&node(5)));
        for offset in 0..fixed_descendants {
            if measure.contains(&node(100 + offset)) {
                count += 1;
            }
        }
        count
    }

    #[test]
    fn parent_width_change_measures_only_children_that_consume_inline() {
        let mut counts = Vec::new();
        for descendants in [4u64, 48] {
            let (mut world, section) = section_with_fixed_subtree(descendants);
            commit_style(&mut world, section, px_box(240.0, 120.0));
            let seeds = seeds_after(&mut world);
            assert!(
                seeds.iter().any(|seed| {
                    seed.node == section
                        && seed.invalidation.affected_axes.intersects(
                            LayoutDependencyFootprint::CONSUMES_PARENT_INLINE_CONSTRAINT,
                        )
                }),
                "a width write must publish an inline parent constraint"
            );
            let frontier = frontier_for(&world, &seeds);
            assert!(
                frontier.measure_nodes().contains(&node(3)),
                "a percent child consumes the inline constraint"
            );
            assert!(
                frontier.measure_nodes().contains(&node(4)),
                "a Fill child consumes the inline constraint"
            );
            let fixed = fixed_subtree_measure_count(&frontier, descendants);
            assert_eq!(
                fixed, 0,
                "fixed descendants are not measured ({descendants} nodes, measured {fixed})"
            );
            counts.push(frontier.measure_nodes().len());
        }
        assert_eq!(
            counts[0], counts[1],
            "growing the fixed subtree must not grow the measure frontier: {counts:?}"
        );
    }

    #[test]
    fn width_constraint_relayouts_text_without_reshaping() {
        let mut world = UiWorld::new();
        let mut queue = MutationQueue::new();
        queue.create(node(1), document(1), NodeKind::Document);
        queue.create(
            node(2),
            document(1),
            NodeKind::Element {
                tag: "section".into(),
            },
        );
        queue.create(node(3), document(1), NodeKind::Text);
        queue.insert(node(1), node(2), None);
        queue.insert(node(2), node(3), None);
        queue.set_style(node(2), style(px_box(320.0, 80.0)));
        queue.set_style(
            node(3),
            style(LayoutStyle {
                width: Some(LengthSpec::Percent(100.0)),
                ..LayoutStyle::default()
            }),
        );
        queue.set_text(
            node(3),
            TextContent {
                value: "a long wrapping line of text that follows the parent width".into(),
            },
        );
        world.commit(queue).unwrap();

        let engine = Arc::new(Mutex::new(NativeTextEngine::new(FontSystem::hermetic())));
        let mut shaper = NanaTextEngineShaper::new(engine);
        let viewport = LayoutViewport::new(400.0, 200.0);
        let mut retained = RetainedLayoutCache::default();

        let work = world.take_system_work();
        world.resolve_styles(&work.style).unwrap();
        world.shape_text(&work.text, &mut shaper).unwrap();
        let emitted = RuntimeLayoutEngine
            .layout_document_with_frontier(
                &world,
                document(1),
                viewport,
                &work.layout_frontier_seeds,
                &mut retained,
                true,
            )
            .unwrap();
        write_boxes(&mut world, &emitted);
        let _ = world.take_system_work();
        world
            .shape_text_for_layout(document(1), &mut shaper)
            .unwrap();

        commit_style(&mut world, node(2), px_box(80.0, 80.0));
        let work = world.take_system_work();
        world.resolve_styles(&work.style).unwrap();
        let shape_before = world
            .nodes
            .text_node(node(3))
            .map(|text| text.revisions.shape);
        let emitted = RuntimeLayoutEngine
            .layout_document_with_frontier(
                &world,
                document(1),
                viewport,
                &work.layout_frontier_seeds,
                &mut retained,
                false,
            )
            .unwrap();
        let text_box = emitted
            .iter()
            .find(|(id, _)| *id == node(3))
            .map(|(_, box_)| *box_)
            .expect("the percent text child is laid out with the new width");
        assert!(
            text_box.width < 100.0,
            "percent width must follow the narrower parent, got {}",
            text_box.width
        );
        write_boxes(&mut world, &emitted);
        let text = world.nodes.text_node(node(3)).expect("text node");
        assert_eq!(text.revisions.shape, shape_before.unwrap());
        assert!(
            text.constraint_moved_without_reshaping(),
            "a width change sets TextDirty::CONSTRAINT and leaves the shaped runs"
        );
        let shaped_before = world.last_work_counters().text_shaped_runs;
        world
            .shape_text_for_layout(document(1), &mut shaper)
            .unwrap();
        let shaped_after = world.last_work_counters().text_shaped_runs;
        let text_work = world.last_text_work_counters();
        assert_eq!(
            shaped_after, shaped_before,
            "a constraint-only width change must not reshape"
        );
        assert_eq!(text_work.text_nodes_shaped, 0);
        assert!(
            text_work.constraint_only_relayouts >= 1,
            "the new width must lay the existing runs out again, got {text_work:?}"
        );
    }

    fn write_boxes(world: &mut UiWorld, emitted: &[(StableNodeId, crate::LayoutBox)]) {
        let mut queue = MutationQueue::new();
        let mut wrote = false;
        for (id, box_) in emitted {
            if world.layout_box(*id) != Some(*box_) {
                queue.write_layout(*id, *box_);
                wrote = true;
            }
        }
        if wrote {
            world.commit(queue).unwrap();
        }
    }

    fn layout_with_seeds(
        world: &UiWorld,
        viewport: LayoutViewport,
        seeds: &[crate::LayoutFrontierSeed],
        retained: &mut RetainedLayoutCache,
        force_full: bool,
    ) -> Vec<(StableNodeId, crate::LayoutBox)> {
        RuntimeLayoutEngine
            .layout_document_with_frontier(
                world,
                document(1),
                viewport,
                seeds,
                retained,
                force_full,
            )
            .unwrap()
    }

    /// Cards in a column that can grow. Each card is a fixed border box around a label.
    fn cards(count: u64) -> (UiWorld, StableNodeId, StableNodeId) {
        let mut world = UiWorld::new();
        let mut queue = MutationQueue::new();
        queue.create(node(1), document(1), NodeKind::Document);
        queue.create(
            node(2),
            document(1),
            NodeKind::Element {
                tag: "column".into(),
            },
        );
        queue.insert(node(1), node(2), None);
        queue.set_style(
            node(2),
            style(LayoutStyle {
                width: Some(LengthSpec::Px(240.0)),
                direction: Some(nana_ui_core::FlexDirection::Column),
                ..LayoutStyle::default()
            }),
        );
        for index in 0..count {
            let card = node(1_000 + index * 2);
            let label = node(1_001 + index * 2);
            queue.create(card, document(1), NodeKind::Element { tag: "card".into() });
            queue.create(label, document(1), NodeKind::Text);
            queue.insert(node(2), card, None);
            queue.insert(card, label, None);
            queue.set_style(card, style(px_box(120.0, 32.0)));
            queue.set_text(label, TextContent { value: "Ok".into() });
        }
        world.commit(queue).unwrap();
        let _ = world.take_system_work();
        let edited = count / 2;
        (world, node(1_000 + edited * 2), node(1_001 + edited * 2))
    }

    fn shape_document(world: &mut UiWorld, shaper: &mut NanaTextEngineShaper) {
        let work = world.take_system_work();
        world.resolve_styles(&work.style).unwrap();
        world.shape_text(&work.text, shaper).unwrap();
    }

    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    struct ExecutionDelta {
        measure_nodes: usize,
        measure_cache_hits: usize,
        measure_cache_misses: usize,
        placement_nodes: usize,
        origin_only_updates: usize,
    }

    fn execution_delta(
        before: nana_ui_core::WorkCounters,
        after: nana_ui_core::WorkCounters,
    ) -> ExecutionDelta {
        ExecutionDelta {
            measure_nodes: after
                .layout_measure_nodes
                .saturating_sub(before.layout_measure_nodes),
            measure_cache_hits: after
                .layout_measure_cache_hits
                .saturating_sub(before.layout_measure_cache_hits),
            measure_cache_misses: after
                .layout_measure_cache_misses
                .saturating_sub(before.layout_measure_cache_misses),
            placement_nodes: after
                .layout_placement_nodes
                .saturating_sub(before.layout_placement_nodes),
            origin_only_updates: after
                .layout_origin_only_updates
                .saturating_sub(before.layout_origin_only_updates),
        }
    }

    struct GateACounts {
        ancestor_measure: usize,
        sibling_placement: usize,
        full_document_fallbacks: usize,
        card_measured: bool,
        metric_generation_bumps: usize,
        execution: ExecutionDelta,
    }

    fn contained_label_edit(count: u64) -> GateACounts {
        let engine = Arc::new(Mutex::new(NativeTextEngine::new(FontSystem::hermetic())));
        let mut shaper = NanaTextEngineShaper::new(engine);
        let viewport = LayoutViewport::new(320.0, 800.0);
        let (mut world, card, label) = cards(count);
        shape_document(&mut world, &mut shaper);
        let mut retained = RetainedLayoutCache::default();
        let emitted = layout_with_seeds(&world, viewport, &[], &mut retained, true);
        write_boxes(&mut world, &emitted);
        let _ = retained.take_intrinsic_counters();
        let _ = world.take_system_work();
        shape_document(&mut world, &mut shaper);
        let _ = world.take_system_work();

        let mut queue = MutationQueue::new();
        queue.set_text(
            label,
            TextContent {
                value: "Longer label".into(),
            },
        );
        world.commit(queue).unwrap();
        let pending = world.take_system_work();
        world.resolve_styles(&pending.style).unwrap();
        world.shape_text(&pending.text, &mut shaper).unwrap();
        let seeded = world.take_system_work();
        let frontier = frontier_for(&world, &seeded.layout_frontier_seeds);
        let before = world.last_work_counters();
        layout_with_seeds(
            &world,
            viewport,
            &seeded.layout_frontier_seeds,
            &mut retained,
            false,
        );
        let stats = retained.frontier_stats(document(1));
        world.record_layout_frontier(stats);
        world.record_layout_execution(retained.execution_stats(document(1)));
        let metric_generation_bumps = retained.take_intrinsic_counters().generation_bumps;
        let after = world.last_work_counters();
        assert_eq!(
            after.layout_full_document_fallbacks - before.layout_full_document_fallbacks,
            stats.full_document_fallbacks,
            "frontier stats are the work-counter authority"
        );
        assert_eq!(
            world.layout_box(card).map(|box_| (box_.width, box_.height)),
            Some((120.0, 32.0)),
            "the fixed card's exported size stays 120x32"
        );
        let mut sibling_placement = 0usize;
        for index in 0..count {
            let other = node(1_000 + index * 2);
            if other == card {
                continue;
            }
            if frontier.placement_nodes().contains(&other) {
                sibling_placement += 1;
            }
        }
        let ancestor_measure = usize::from(frontier.measure_nodes().contains(&node(1)))
            + usize::from(frontier.measure_nodes().contains(&node(2)));
        GateACounts {
            ancestor_measure,
            sibling_placement,
            full_document_fallbacks: stats.full_document_fallbacks,
            card_measured: frontier.measure_nodes().contains(&card),
            metric_generation_bumps,
            execution: execution_delta(before, after),
        }
    }

    #[test]
    fn fixed_card_label_edit_does_not_measure_ancestors_or_place_siblings() {
        let mut counts = Vec::new();
        for count in [48u64, 160] {
            let gate = contained_label_edit(count);
            assert_eq!(
                gate.ancestor_measure, 0,
                "{count} cards: ancestors entered the measure frontier"
            );
            assert!(
                !gate.card_measured,
                "{count} cards: the fixed card was remeasured for an inner label"
            );
            assert_eq!(
                gate.sibling_placement, 0,
                "{count} cards: siblings entered placement"
            );
            assert_eq!(gate.full_document_fallbacks, 0);
            counts.push((
                gate.ancestor_measure,
                gate.sibling_placement,
                gate.full_document_fallbacks,
                gate.metric_generation_bumps,
                gate.execution,
            ));
            assert!(
                gate.execution.measure_nodes + gate.execution.placement_nodes > 0,
                "{count} cards: the edit produced no measure or placement work"
            );
            assert!(
                gate.metric_generation_bumps > 0,
                "{count} cards: the label's new metrics did not bump generation"
            );
        }
        assert_eq!(
            counts[0], counts[1],
            "structural counts must be constant as the card list grows: {counts:?}"
        );
    }

    fn rows(count: u64) -> UiWorld {
        let mut world = UiWorld::new();
        let mut queue = MutationQueue::new();
        queue.create(node(1), document(1), NodeKind::Document);
        queue.create(
            node(2),
            document(1),
            NodeKind::Element {
                tag: "column".into(),
            },
        );
        queue.insert(node(1), node(2), None);
        queue.set_style(
            node(2),
            style(LayoutStyle {
                width: Some(LengthSpec::Px(200.0)),
                direction: Some(nana_ui_core::FlexDirection::Column),
                ..LayoutStyle::default()
            }),
        );
        for index in 0..count {
            let row = node(3_000 + index);
            queue.create(row, document(1), NodeKind::Element { tag: "row".into() });
            queue.insert(node(2), row, None);
            queue.set_style(row, style(px_box(180.0, 20.0)));
        }
        world.commit(queue).unwrap();
        let _ = world.take_system_work();
        world
    }

    struct GateBCounts {
        children_measured: usize,
        suffixes_replayed: usize,
        following_measured: usize,
        prefix_emitted: bool,
        full_document_fallbacks: usize,
        execution: ExecutionDelta,
    }

    fn late_row_height(count: u64) -> GateBCounts {
        let viewport = LayoutViewport::new(240.0, 400.0);
        let mut world = rows(count);
        let edited_index = count.saturating_sub(8);
        let following = node(3_000 + edited_index + 1);
        let following_label = node(4_000 + edited_index);
        let mut queue = MutationQueue::new();
        queue.create(following_label, document(1), NodeKind::Text);
        queue.insert(following, following_label, None);
        queue.set_style(following_label, style(px_box(40.0, 12.0)));
        queue.set_text(
            following_label,
            TextContent {
                value: "tail".into(),
            },
        );
        world.commit(queue).unwrap();
        let _ = world.take_system_work();
        let mut retained = RetainedLayoutCache::default();
        let emitted = layout_with_seeds(&world, viewport, &[], &mut retained, true);
        write_boxes(&mut world, &emitted);
        let _ = world.take_system_work();

        // The full pass records no measure plan. One scoped edit builds it.
        let primer = node(3_000);
        let mut primed = (*world.node_style(primer).unwrap().layout).clone();
        primed.height = Some(LengthSpec::Px(21.0));
        commit_style(&mut world, primer, primed);
        let work = world.take_system_work();
        world.resolve_styles(&work.style).unwrap();
        let emitted = layout_with_seeds(
            &world,
            viewport,
            &work.layout_frontier_seeds,
            &mut retained,
            false,
        );
        write_boxes(&mut world, &emitted);
        let _ = world.take_system_work();

        let edited = node(3_000 + edited_index);
        let mut layout = (*world.node_style(edited).unwrap().layout).clone();
        layout.height = Some(LengthSpec::Px(36.0));
        commit_style(&mut world, edited, layout);
        let work = world.take_system_work();
        world.resolve_styles(&work.style).unwrap();
        let frontier = frontier_for(&world, &work.layout_frontier_seeds);
        let before = world.last_work_counters();
        let emitted = layout_with_seeds(
            &world,
            viewport,
            &work.layout_frontier_seeds,
            &mut retained,
            false,
        );
        let stats = retained.frontier_stats(document(1));
        world.record_layout_frontier(stats);
        world.record_layout_execution(retained.execution_stats(document(1)));
        let execution = execution_delta(before, world.last_work_counters());
        let following_measured = (edited_index + 1..count)
            .filter(|index| frontier.measure_nodes().contains(&node(3_000 + index)))
            .count();
        assert!(
            frontier.placement_nodes().contains(&following),
            "the next row stays on the placement frontier"
        );
        // A descendant that only moves moves with its row: placing the row at
        // its new origin places it, so it is on neither frontier.
        assert!(!frontier.placement_nodes().contains(&following_label));
        assert!(!frontier.measure_nodes().contains(&following_label));
        assert!(!frontier.measure_nodes().contains(&node(3_000)));
        assert!(!frontier.placement_nodes().contains(&node(3_000)));
        let prefix_emitted = emitted.iter().any(|(id, _)| *id == node(3_000));
        GateBCounts {
            children_measured: retained.execution_stats(document(1)).children_measured,
            suffixes_replayed: retained.execution_stats(document(1)).suffixes_replayed,
            following_measured,
            prefix_emitted,
            full_document_fallbacks: stats.full_document_fallbacks,
            execution,
        }
    }

    #[test]
    fn late_row_height_replays_suffix_without_measuring_followers() {
        let mut measured = Vec::new();
        for count in [64u64, 256] {
            let gate = late_row_height(count);
            assert_eq!(
                gate.following_measured, 0,
                "{count} rows: following siblings entered the measure frontier"
            );
            assert!(
                gate.suffixes_replayed > 0,
                "{count} rows: the column did not replay its suffix"
            );
            assert!(
                !gate.prefix_emitted,
                "{count} rows: prefix placement was rebuilt"
            );
            assert_eq!(gate.full_document_fallbacks, 0);
            assert!(
                gate.children_measured <= 4,
                "{count} rows: unaffected children were measured ({})",
                gate.children_measured
            );
            assert!(
                gate.execution.origin_only_updates > 0,
                "{count} rows: the suffix move did not record an origin-only placement"
            );
            assert!(
                gate.execution.placement_nodes > 0,
                "{count} rows: placement wrote no boxes"
            );
            measured.push((gate.children_measured, gate.execution));
        }
        assert_eq!(
            measured[0], measured[1],
            "measure work must not grow with the row count: {measured:?}"
        );
    }

    #[test]
    fn equivalent_recompute_does_not_publish_layout_geometry() {
        let mut world = UiWorld::new();
        let mut queue = MutationQueue::new();
        queue.create(node(1), document(1), NodeKind::Document);
        queue.create(
            node(2),
            document(1),
            NodeKind::Element {
                tag: "section".into(),
            },
        );
        queue.create(
            node(3),
            document(1),
            NodeKind::Element {
                tag: "fixed".into(),
            },
        );
        queue.insert(node(1), node(2), None);
        queue.insert(node(1), node(3), None);
        queue.set_style(node(2), style(px_box(120.0, 32.0)));
        queue.set_style(
            node(3),
            style(LayoutStyle {
                position: nana_ui_core::PositionSpec::Fixed,
                width: Some(LengthSpec::Px(40.0)),
                height: Some(LengthSpec::Px(16.0)),
                offset_top: Some(LengthSpec::Px(4.0)),
                offset_left: Some(LengthSpec::Px(6.0)),
                ..LayoutStyle::default()
            }),
        );
        world.commit(queue).unwrap();
        let _ = world.take_system_work();
        let viewport = LayoutViewport::new(200.0, 100.0);
        let mut retained = RetainedLayoutCache::default();
        let emitted = layout_with_seeds(&world, viewport, &[], &mut retained, true);
        write_boxes(&mut world, &emitted);
        let _ = retained.take_intrinsic_counters();
        let _ = world.take_system_work();
        let ids = emitted.iter().map(|(id, _)| *id).collect::<Vec<_>>();
        world.publish_layout_results(&ids, crate::LayoutResultSource::RuntimeLayout);
        let generation = world.layout_generation();
        let section_ptr = world
            .layout_results
            .get(&node(2))
            .map(|result| Arc::as_ptr(&result.local))
            .expect("section result");
        let fixed_ptr = world
            .layout_results
            .get(&node(3))
            .map(|result| Arc::as_ptr(&result.local))
            .expect("fixed result");

        // A definite min-width below the used width recomputes the section and
        // leaves the border box unchanged.
        let mut layout = (*world.node_style(node(2)).unwrap().layout).clone();
        layout.min_width = Some(LengthSpec::Px(10.0));
        commit_style(&mut world, node(2), layout);
        let work = world.take_system_work();
        world.resolve_styles(&work.style).unwrap();
        let emitted = layout_with_seeds(
            &world,
            viewport,
            &work.layout_frontier_seeds,
            &mut retained,
            false,
        );
        assert_eq!(
            retained.take_intrinsic_counters().generation_bumps,
            0,
            "a bit-equivalent recompute must not bump retained metric generation"
        );
        write_boxes(&mut world, &emitted);
        let _ = world.take_system_work();
        let ids = emitted.iter().map(|(id, _)| *id).collect::<Vec<_>>();
        let before = world.layout_box(node(2));
        let counters_before = world.last_work_counters();
        world.publish_layout_results(&ids, crate::LayoutResultSource::RuntimeLayout);
        let counters_after = world.last_work_counters();

        assert_eq!(
            (world.layout_generation(), world.layout_box(node(2))),
            (generation, before),
            "a bit-equivalent recompute must not bump layout generation"
        );
        assert_eq!(
            world
                .layout_results
                .get(&node(2))
                .map(|result| Arc::as_ptr(&result.local)),
            Some(section_ptr),
            "the section result object must stay the one already published"
        );
        assert_eq!(
            world
                .layout_results
                .get(&node(3))
                .map(|result| Arc::as_ptr(&result.local)),
            Some(fixed_ptr),
            "an untouched fixed-position result must not be republished"
        );
        assert!(world.layout_result(node(2)).is_some());
        assert_eq!(world.layout_result(node(2)).unwrap().bounds.width, 120.0);
        assert_eq!(
            counters_after.layout_result_changed, counters_before.layout_result_changed,
            "a bit-equivalent recompute must not count a changed layout result"
        );
        assert_eq!(
            counters_after.layout_delta_commits, counters_before.layout_delta_commits,
            "a bit-equivalent recompute must not commit a layout delta"
        );
        assert!(
            counters_after.layout_result_reused > counters_before.layout_result_reused,
            "keeping the published object must count as reuse"
        );

        // position:fixed always misses the cheap geometry check. Publishing the
        // same boxes again must still keep the Arc.
        let fixed_before = world.last_work_counters();
        world.publish_layout_results(&[node(3)], crate::LayoutResultSource::RuntimeLayout);
        let fixed_after = world.last_work_counters();
        assert_eq!(world.layout_generation(), generation);
        assert_eq!(
            world
                .layout_results
                .get(&node(3))
                .map(|result| Arc::as_ptr(&result.local)),
            Some(fixed_ptr)
        );
        assert_eq!(
            fixed_after.layout_result_changed,
            fixed_before.layout_result_changed
        );
        assert_eq!(
            fixed_after.layout_delta_commits,
            fixed_before.layout_delta_commits
        );
        assert!(fixed_after.layout_result_reused > fixed_before.layout_result_reused);
    }

    /// Rows of a fixed-width column, each holding a label.
    fn labelled_rows(count: u64) -> UiWorld {
        let mut world = rows(count);
        let mut queue = MutationQueue::new();
        for index in 0..count {
            let label = node(4_000 + index);
            queue.create(label, document(1), NodeKind::Text);
            queue.insert(node(3_000 + index), label, None);
            queue.set_style(label, style(px_box(40.0, 12.0)));
            queue.set_text(
                label,
                TextContent {
                    value: "row".into(),
                },
            );
        }
        world.commit(queue).unwrap();
        let _ = world.take_system_work();
        world
    }

    struct AppendCounts {
        measured: usize,
        placed: usize,
        old_labels_reached: usize,
    }

    fn append_row(count: u64) -> AppendCounts {
        let mut world = labelled_rows(count);
        let row = node(9_000);
        let label = node(9_001);
        let mut queue = MutationQueue::new();
        queue.create(row, document(1), NodeKind::Element { tag: "row".into() });
        queue.insert(node(2), row, None);
        queue.set_style(row, style(px_box(180.0, 20.0)));
        queue.create(label, document(1), NodeKind::Text);
        queue.insert(row, label, None);
        queue.set_style(label, style(px_box(40.0, 12.0)));
        world.commit(queue).unwrap();
        let seeds = seeds_after(&mut world);
        let frontier = frontier_for(&world, &seeds);
        assert!(
            frontier.measure_nodes().contains(&row),
            "the new row is laid out"
        );
        assert!(frontier.measure_nodes().contains(&label), "and its subtree");
        assert!(
            frontier.measure_nodes().contains(&node(2)),
            "the column re-measures"
        );
        AppendCounts {
            measured: frontier.measure_nodes().len(),
            placed: frontier.placement_nodes().len(),
            old_labels_reached: (0..count)
                .filter(|index| frontier.contains(node(4_000 + index)))
                .count(),
        }
    }

    #[test]
    fn appending_a_row_lays_out_the_row_not_the_whole_column() {
        let small = append_row(32);
        let large = append_row(256);
        assert_eq!(
            small.measured, large.measured,
            "measure work follows the new row and its metric path, not the column"
        );
        assert_eq!(
            large.old_labels_reached, 0,
            "existing rows' content is untouched"
        );
        // Rows after the insertion point move; here there are none, and the
        // existing rows keep their place.
        assert_eq!(small.placed, large.placed);
    }

    /// The fallback counters report what they name: a subtree laid out in full
    /// for an unknown cause, and such a subtree that is the whole document. A row
    /// that just arrived is laid out in full too, but that is its first layout.
    #[test]
    fn only_unknown_causes_count_as_fallbacks() {
        let viewport = LayoutViewport::new(240.0, 400.0);
        let mut world = labelled_rows(8);
        let mut retained = RetainedLayoutCache::default();
        let emitted = layout_with_seeds(&world, viewport, &[], &mut retained, true);
        write_boxes(&mut world, &emitted);
        let _ = world.take_system_work();

        let mut queue = MutationQueue::new();
        queue.create(
            node(9_000),
            document(1),
            NodeKind::Element { tag: "row".into() },
        );
        queue.insert(node(2), node(9_000), None);
        queue.set_style(node(9_000), style(px_box(180.0, 20.0)));
        world.commit(queue).unwrap();
        let seeds = seeds_after(&mut world);
        let emitted = layout_with_seeds(&world, viewport, &seeds, &mut retained, false);
        write_boxes(&mut world, &emitted);
        let _ = world.take_system_work();
        let stats = retained.frontier_stats(document(1));
        assert_eq!(stats.local_subtree_fallbacks, 0);
        assert_eq!(stats.full_document_fallbacks, 0);

        world.mark_layout(node(3_000));
        let seeds = seeds_after(&mut world);
        let emitted = layout_with_seeds(&world, viewport, &seeds, &mut retained, false);
        write_boxes(&mut world, &emitted);
        let _ = world.take_system_work();
        let stats = retained.frontier_stats(document(1));
        assert_eq!(stats.local_subtree_fallbacks, 1);
        assert_eq!(stats.full_document_fallbacks, 0);

        world.mark_layout(node(2));
        let seeds = seeds_after(&mut world);
        layout_with_seeds(&world, viewport, &seeds, &mut retained, false);
        let stats = retained.frontier_stats(document(1));
        assert_eq!(stats.local_subtree_fallbacks, 1);
        assert_eq!(stats.full_document_fallbacks, 1);
    }
}

mod scaling_matrix {
    //! Issue #259: the performance contract of incremental reflow.
    //!
    //! One fixture, the scaling matrix. A page of groups; each group's body holds
    //! the formatting contexts the scenarios edit -- a sequential list of rows, a
    //! fixed card with a label, a fixed flex row, a fixed grid with auto tracks
    //! and a fixed inline formatting context. Groups repeat to 1k, 10k and 100k
    //! nodes; the body sits directly in its group (shallow) or under eight
    //! wrappers; the page is LTR, RTL or vertical-rl. A case makes one edit in 1,
    //! 8 or 100 groups at the head, middle or tail of the page, in one frame.
    //!
    //! The gates read [`WorkCounters`]:
    //!
    //! - paint-only: every layout counter is zero;
    //! - local-contained: the frontier, the edges it walked, the boxes placed,
    //!   the children walked and the subtrees measured are the same at 1k, 10k
    //!   and 100k nodes;
    //! - sequential: a frame places the boxes that moved and the way down to the
    //!   edit, and measures the same subtrees at every size. Only the edited
    //!   containers replay a suffix, or miss and record their plans again, the
    //!   same number at every size and no more than boxes moved: a container that
    //!   only moves takes its plan along. At the flow end whose growth moves
    //!   nothing after it, all of that is a constant;
    //! - text: a label at the head of a page, in a column it does not fill,
    //!   takes a longer string that still fits: the frame lays out once, as for
    //!   a label that cannot wrap, and nothing below the label moves, at 1k and
    //!   10k nodes. With the page's groups directly after the column, the frame
    //!   builds, walks and places the same at both sizes; a string that wraps
    //!   grows the column and moves every group after it. Each group takes its
    //!   cards along at their offsets in it: the frontier that found them, the
    //!   children walked and the plans missed are still the same;
    //! - child list: a box arrives in that column, or in the page before it, and
    //!   leaves again. Every box after it moves once each way, none of them on
    //!   the frontier, found by the same frontier, with the same children walked
    //!   and plans missed, at 1k and 10k nodes; in a column of a fixed height
    //!   the whole frame costs the same. A box at either end of the page itself
    //!   leaves and arrives without a subtree walked to measure it, the page's
    //!   included, nor a group walked to move it; at the tail the frame costs
    //!   the same at both sizes but for the page's own result;
    //! - batch: a hundred seeds walk each edge of their union closure at most
    //!   twice and cost no more than a hundred single edits;
    //! - memory: 10k edits on 100k nodes leave the retained cache what the tree
    //!   gives it, keep no full-layout copy, and no frame's scratch outgrows one
    //!   edit's closure;
    //! - virtualization: a row of a 100k and of a 1M row virtual list growing
    //!   costs the list its window, the same at both sizes (Issue #262);
    //! - responsive: a container resized within its rules' buckets, across one
    //!   breakpoint and back, and eight nested containers resized across
    //!   theirs, cost the same at 1k, 10k and 100k nodes (Issue #265);
    //! - locale: forty localized labels at the end of a window switch from
    //!   American English to Chinese, with their language named to British
    //!   English (half of them reading the same), and as a scope of their own to
    //!   Arabic, right to left, at the same cost in 1k, 10k and 100k node
    //!   windows; a localized virtual list switches at the same cost at 100k and
    //!   1M rows (Issues #268, #269);
    //! - formatting: a counter whose count changes, a catalog update of one
    //!   message, and a switch to a locale whose formatters are not built yet,
    //!   cost the same at 10k and 100k nodes (Issue #270).
    //!
    //! Every page ends against the cold oracle ([`crate::world::reflow_oracle`]); 1k
    //! pages also run the per-pass guard on every frame.

    use std::collections::HashSet;
    use std::sync::Arc;

    use nana_ui_core::{
        DirSpec, DisplaySpec, FlexDirection, GridTrack, LayoutStyle, LengthSpec, SemanticColorRole,
        WorkCounters, WritingModeSpec,
    };

    use crate::layout_engine::verify::skip_layout_verify;
    use crate::world::reflow_oracle::{
        assert_matches_cold, bundled_face_shaper, node, product_frame, resize_frame, styled,
    };
    use crate::world::{DocumentId, NodeKind, StableNodeId, UiWorld};
    use crate::{
        AppContext, LayoutBox, LayoutFrontierStats, LayoutViewport, MutationQueue,
        NanaTextEngineShaper, NodeStyle, TextContent,
    };

    fn viewport() -> LayoutViewport {
        LayoutViewport::new(400.0, 600.0)
    }

    fn column() -> NodeStyle {
        styled(LayoutStyle {
            direction: Some(FlexDirection::Column),
            ..LayoutStyle::default()
        })
    }

    const ROWS: u64 = 8;
    const ITEMS: u64 = 4;
    const CELLS: u64 = 6;
    const ATOMS: u64 = 3;

    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
    enum Depth {
        Shallow,
        Nested,
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
    enum Writing {
        Ltr,
        Rtl,
        VerticalRl,
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
    enum Scenario {
        PaintOnly,
        FixedContent,
        Sequential,
        ParentConstraint,
        Flex,
        Grid,
        Inline,
        /// The card names a language for its label (Issue #260): the label
        /// shapes again in it, and lays out again only if its metrics moved.
        Language,
        /// The card sets a typography scale for its label (Issue #266): the label
        /// lays out again at the new size inside the fixed card.
        Typography,
    }

    impl Scenario {
        const ALL: [Self; 9] = [
            Self::PaintOnly,
            Self::FixedContent,
            Self::Sequential,
            Self::ParentConstraint,
            Self::Flex,
            Self::Grid,
            Self::Inline,
            Self::Language,
            Self::Typography,
        ];
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
    enum Position {
        Head,
        Middle,
        Tail,
    }

    impl Position {
        const ALL: [Self; 3] = [Self::Head, Self::Middle, Self::Tail];
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    struct Shape {
        depth: Depth,
        writing: Writing,
    }

    impl Shape {
        /// The shallow left-to-right page most single-shape gates run on.
        const SHALLOW_LTR: Self = Self {
            depth: Depth::Shallow,
            writing: Writing::Ltr,
        };

        fn wrappers(self) -> u64 {
            match self.depth {
                Depth::Shallow => 0,
                Depth::Nested => 8,
            }
        }

        fn group_nodes(self) -> u64 {
            self.wrappers() + 9 + ROWS + ITEMS + CELLS + ATOMS
        }

        fn group(self, group: u64) -> StableNodeId {
            node(3 + group * self.group_nodes())
        }

        fn wrapper(self, group: u64, depth: u64) -> StableNodeId {
            node(self.group(group).get() + 1 + depth)
        }

        fn body(self, group: u64) -> StableNodeId {
            node(self.group(group).get() + 1 + self.wrappers())
        }

        fn list(self, group: u64) -> StableNodeId {
            node(self.body(group).get() + 1)
        }

        fn row(self, group: u64, row: u64) -> StableNodeId {
            node(self.list(group).get() + 1 + row)
        }

        fn card(self, group: u64) -> StableNodeId {
            node(self.list(group).get() + 1 + ROWS)
        }

        fn label(self, group: u64) -> StableNodeId {
            node(self.card(group).get() + 1)
        }

        fn flex(self, group: u64) -> StableNodeId {
            node(self.label(group).get() + 1)
        }

        fn item(self, group: u64, item: u64) -> StableNodeId {
            node(self.flex(group).get() + 1 + item)
        }

        fn grid(self, group: u64) -> StableNodeId {
            node(self.flex(group).get() + 1 + ITEMS)
        }

        fn cell(self, group: u64, cell: u64) -> StableNodeId {
            node(self.grid(group).get() + 1 + cell)
        }

        fn para(self, group: u64) -> StableNodeId {
            node(self.grid(group).get() + 1 + CELLS)
        }

        fn atom(self, group: u64, atom: u64) -> StableNodeId {
            node(self.para(group).get() + 1 + atom)
        }

        fn text(self, group: u64) -> StableNodeId {
            node(self.para(group).get() + 1 + ATOMS)
        }

        /// The node `scenario` edits in `group`.
        fn target(self, scenario: Scenario, group: u64) -> StableNodeId {
            match scenario {
                Scenario::PaintOnly
                | Scenario::ParentConstraint
                | Scenario::Language
                | Scenario::Typography => self.card(group),
                Scenario::FixedContent => self.label(group),
                Scenario::Sequential => self.row(group, 0),
                Scenario::Flex => self.item(group, 0),
                Scenario::Grid => self.cell(group, 0),
                Scenario::Inline => self.atom(group, 0),
            }
        }

        /// A box `inline` long on the page's inline axis and `block` long on its
        /// block axis.
        fn sized(self, inline: Option<f32>, block: Option<f32>) -> LayoutStyle {
            let (width, height) = if self.writing == Writing::VerticalRl {
                (block, inline)
            } else {
                (inline, block)
            };
            LayoutStyle {
                width: width.map(LengthSpec::Px),
                height: height.map(LengthSpec::Px),
                ..LayoutStyle::default()
            }
        }

        fn page_style(self) -> NodeStyle {
            let mut layout = self.sized(Some(320.0), None);
            layout.direction = Some(FlexDirection::Column);
            match self.writing {
                Writing::Ltr => {}
                Writing::Rtl => layout.dir = Some(DirSpec::Rtl),
                Writing::VerticalRl => layout.writing_mode = Some(WritingModeSpec::VerticalRl),
            }
            styled(layout)
        }

        fn row_style(self, grown: bool) -> NodeStyle {
            styled(self.sized(Some(300.0), Some(if grown { 30.0 } else { 20.0 })))
        }

        fn card_style(self, narrowed: bool, painted: bool) -> NodeStyle {
            let mut layout = self.sized(Some(if narrowed { 180.0 } else { 200.0 }), Some(40.0));
            layout.direction = Some(FlexDirection::Row);
            NodeStyle {
                layout: Arc::new(layout),
                background: painted.then_some(SemanticColorRole::Selected),
                ..NodeStyle::default()
            }
        }

        fn flex_style(self) -> NodeStyle {
            let mut layout = self.sized(Some(300.0), Some(20.0));
            layout.direction = Some(FlexDirection::Row);
            styled(layout)
        }

        /// Growth is shared by `flex-grow`; doubling one item's share moves
        /// every item on the line.
        fn item_style(self, grown: bool) -> NodeStyle {
            let mut layout = self.sized(None, Some(20.0));
            layout.flex_grow = Some(if grown { 2.0 } else { 1.0 });
            styled(layout)
        }

        fn grid_style(self) -> NodeStyle {
            let mut layout = self.sized(Some(300.0), Some(60.0));
            layout.display = Some(DisplaySpec::Grid);
            layout.grid_columns = Some(vec![GridTrack::Auto; 3]);
            styled(layout)
        }

        fn cell_style(self, grown: bool) -> NodeStyle {
            styled(self.sized(Some(if grown { 50.0 } else { 30.0 }), Some(20.0)))
        }

        fn para_style(self) -> NodeStyle {
            let mut layout = self.sized(Some(300.0), Some(60.0));
            layout.display = Some(DisplaySpec::Block);
            styled(layout)
        }

        fn atom_style(self, grown: bool) -> NodeStyle {
            let mut layout = self.sized(Some(if grown { 100.0 } else { 80.0 }), Some(20.0));
            layout.display = Some(DisplaySpec::InlineBlock);
            styled(layout)
        }
    }

    fn inline_text_style() -> NodeStyle {
        styled(LayoutStyle {
            display: Some(DisplaySpec::Inline),
            ..LayoutStyle::default()
        })
    }

    fn card_language(edited: bool) -> Option<crate::LanguageTag> {
        edited.then(|| crate::LanguageTag::new("ja").unwrap())
    }

    fn card_text_scale(edited: bool) -> Option<f32> {
        edited.then_some(1.25)
    }

    fn label_text(edited: bool) -> &'static str {
        if edited { "seventeen" } else { "one" }
    }

    /// The (scenario, group) edits in effect. Every node is built from this set,
    /// so a page built cold from the same set has the same inputs.
    #[derive(Debug, Default, Clone)]
    struct Edits(HashSet<(Scenario, u64)>);

    impl Edits {
        fn on(&self, scenario: Scenario, group: u64) -> bool {
            self.0.contains(&(scenario, group))
        }

        fn toggle(&mut self, scenario: Scenario, group: u64) {
            if !self.0.remove(&(scenario, group)) {
                self.0.insert((scenario, group));
            }
        }
    }

    /// A page's mutations, each node at the id its [`Shape`] gives it.
    struct Tree {
        queue: MutationQueue,
        document: DocumentId,
    }

    impl Tree {
        fn new(document: DocumentId) -> Self {
            Self {
                queue: MutationQueue::new(),
                document,
            }
        }

        fn element(&mut self, parent: StableNodeId, id: StableNodeId, tag: &str, style: NodeStyle) {
            self.queue
                .create(id, self.document, NodeKind::Element { tag: tag.into() });
            self.queue.insert(parent, id, None);
            self.queue.set_style(id, style);
        }

        fn text(&mut self, parent: StableNodeId, id: StableNodeId, value: &str) {
            self.queue.create(id, self.document, NodeKind::Text);
            self.queue.insert(parent, id, None);
            self.queue.set_text(
                id,
                TextContent {
                    value: value.into(),
                },
            );
        }
    }

    fn build_group(tree: &mut Tree, shape: Shape, edits: &Edits, group: u64) {
        let mut parent = shape.group(group);
        tree.element(node(2), parent, "group", column());
        for depth in 0..shape.wrappers() {
            let wrapper = shape.wrapper(group, depth);
            tree.element(parent, wrapper, "wrapper", column());
            parent = wrapper;
        }
        let body = shape.body(group);
        tree.element(parent, body, "body", column());
        tree.element(body, shape.list(group), "list", column());
        for row in 0..ROWS {
            let style = shape.row_style(row == 0 && edits.on(Scenario::Sequential, group));
            tree.element(shape.list(group), shape.row(group, row), "row", style);
        }
        let card = shape.card(group);
        let style = shape.card_style(
            edits.on(Scenario::ParentConstraint, group),
            edits.on(Scenario::PaintOnly, group),
        );
        tree.element(body, card, "card", style);
        if edits.on(Scenario::Language, group) {
            tree.queue.set_language(card, card_language(true));
        }
        if edits.on(Scenario::Typography, group) {
            tree.queue.set_text_scale(card, card_text_scale(true));
        }
        let label = label_text(edits.on(Scenario::FixedContent, group));
        tree.text(card, shape.label(group), label);
        tree.element(body, shape.flex(group), "flex", shape.flex_style());
        for item in 0..ITEMS {
            let style = shape.item_style(item == 0 && edits.on(Scenario::Flex, group));
            tree.element(shape.flex(group), shape.item(group, item), "item", style);
        }
        tree.element(body, shape.grid(group), "grid", shape.grid_style());
        for cell in 0..CELLS {
            let style = shape.cell_style(cell == 0 && edits.on(Scenario::Grid, group));
            tree.element(shape.grid(group), shape.cell(group, cell), "cell", style);
        }
        tree.element(body, shape.para(group), "para", shape.para_style());
        for atom in 0..ATOMS {
            let style = shape.atom_style(atom == 0 && edits.on(Scenario::Inline, group));
            tree.element(shape.para(group), shape.atom(group, atom), "atom", style);
        }
        tree.text(shape.para(group), shape.text(group), "one two");
        tree.queue.set_style(shape.text(group), inline_text_style());
    }

    /// Whether a page checks every layout pass against a full layout. Pages of
    /// 10k nodes and more check against the cold oracle at the end instead: a
    /// full layout per frame would cost more than the frames under test.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum Guard {
        EveryPass,
        ColdOnly,
    }

    struct Page {
        shape: Shape,
        groups: u64,
        edits: Edits,
        guard: Guard,
        context: AppContext,
        document: DocumentId,
        shaper: NanaTextEngineShaper,
    }

    impl Page {
        fn new(shape: Shape, nodes: u64, guard: Guard) -> Self {
            Self::with_edits(shape, nodes, Edits::default(), guard)
        }

        /// About `nodes` nodes with `edits` in effect, laid out in one frame.
        fn with_edits(shape: Shape, nodes: u64, edits: Edits, guard: Guard) -> Self {
            let groups = (nodes.saturating_sub(2) / shape.group_nodes()).max(1);
            let document = DocumentId::new(1).unwrap();
            let mut tree = Tree::new(document);
            tree.queue.create(node(1), document, NodeKind::Document);
            tree.element(node(1), node(2), "page", shape.page_style());
            for group in 0..groups {
                build_group(&mut tree, shape, &edits, group);
            }
            let mut context = AppContext::new();
            context.compat_world_mut().commit(tree.queue).unwrap();
            let mut page = Self {
                shape,
                groups,
                edits,
                guard,
                context,
                document,
                shaper: bundled_face_shaper(),
            };
            page.frame();
            page
        }

        /// The same inputs, laid out once from nothing.
        fn cold(&self) -> Self {
            let nodes = self.groups * self.shape.group_nodes() + 2;
            Self::with_edits(self.shape, nodes, self.edits.clone(), Guard::ColdOnly)
        }

        fn assert_matches_cold(&mut self) {
            let mut cold = self.cold();
            assert_matches_cold(&mut self.context, &mut cold.context, self.document);
        }

        fn world(&self) -> &UiWorld {
            self.context.world()
        }

        /// `seeds` consecutive groups at `position`.
        fn groups_at(&self, position: Position, seeds: u64) -> Vec<u64> {
            let seeds = seeds.min(self.groups);
            let start = match position {
                Position::Head => 0,
                Position::Middle => (self.groups - seeds) / 2,
                Position::Tail => self.groups - seeds,
            };
            (start..start + seeds).collect()
        }

        /// Toggle `scenario`'s edit in every group of `groups`, as one commit.
        fn edit(&mut self, scenario: Scenario, groups: &[u64]) {
            let shape = self.shape;
            let mut queue = MutationQueue::new();
            for &group in groups {
                self.edits.toggle(scenario, group);
                let on = self.edits.on(scenario, group);
                let target = shape.target(scenario, group);
                match scenario {
                    Scenario::PaintOnly | Scenario::ParentConstraint => queue.set_style(
                        target,
                        shape.card_style(
                            self.edits.on(Scenario::ParentConstraint, group),
                            self.edits.on(Scenario::PaintOnly, group),
                        ),
                    ),
                    Scenario::FixedContent => queue.set_text(
                        target,
                        TextContent {
                            value: label_text(on).into(),
                        },
                    ),
                    Scenario::Sequential => queue.set_style(target, shape.row_style(on)),
                    Scenario::Flex => queue.set_style(target, shape.item_style(on)),
                    Scenario::Grid => queue.set_style(target, shape.cell_style(on)),
                    Scenario::Inline => queue.set_style(target, shape.atom_style(on)),
                    Scenario::Language => queue.set_language(target, card_language(on)),
                    Scenario::Typography => queue.set_text_scale(target, card_text_scale(on)),
                }
            }
            self.context.compat_world_mut().commit(queue).unwrap();
        }

        fn frame(&mut self) -> WorkCounters {
            let _unguarded = (self.guard == Guard::ColdOnly).then(skip_layout_verify);
            product_frame(
                &mut self.context,
                self.document,
                viewport(),
                &mut self.shaper,
            )
        }

        /// One frame of a window resize to `viewport`.
        fn resize(&mut self, viewport: LayoutViewport) -> WorkCounters {
            let _unguarded = (self.guard == Guard::ColdOnly).then(skip_layout_verify);
            resize_frame(&mut self.context, self.document, viewport, &mut self.shaper)
        }

        fn boxes(&self) -> Vec<Option<LayoutBox>> {
            let world = self.world();
            world
                .document_order(self.document)
                .into_iter()
                .map(|id| world.layout_box(id))
                .collect()
        }
    }

    /// What one frame cost, in the units the gates name.
    #[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
    struct Cost {
        seeds: usize,
        frontier_measure: usize,
        frontier_placement: usize,
        contexts: usize,
        edges: usize,
        /// Nodes that computed a used size: subtree walks, and fixed-size boxes
        /// resolving their own style.
        measured: usize,
        placed: usize,
        /// Boxes a container that only moved took along without placing them:
        /// counted by the gates from the boxes (`carried`), as nothing visits
        /// them to count.
        translated: usize,
        origin_only: usize,
        children_walked: usize,
        /// Children result publication checked or rebuilt.
        result_children: usize,
        full_subtrees: usize,
        plan_misses: usize,
        /// Plans recorded again over an old one for the same container and
        /// constraint.
        plan_rebuilds: usize,
        /// Sequential containers that replayed only the part after a change.
        suffixes_replayed: usize,
        local_fallbacks: usize,
        full_fallbacks: usize,
    }

    impl From<WorkCounters> for Cost {
        fn from(counters: WorkCounters) -> Self {
            Self {
                seeds: counters.layout_frontier_seeds,
                frontier_measure: counters.layout_frontier_nodes_measure,
                frontier_placement: counters.layout_frontier_nodes_placement,
                contexts: counters.layout_frontier_contexts,
                edges: counters.layout_dependency_edges_visited,
                measured: counters.layout_measure_nodes,
                placed: counters.layout_placement_nodes,
                translated: counters.layout_translated_nodes,
                origin_only: counters.layout_origin_only_updates,
                children_walked: counters.layout_children_measured,
                result_children: counters.layout_result_children_visited,
                full_subtrees: counters.intrinsic_measure_full_subtrees,
                plan_misses: counters.layout_plan_misses,
                plan_rebuilds: counters.layout_plan_rebuilds,
                suffixes_replayed: counters.layout_suffixes_replayed,
                local_fallbacks: counters.layout_local_subtree_fallbacks,
                full_fallbacks: counters.layout_full_document_fallbacks,
            }
        }
    }

    impl Cost {
        /// Everything but `measured`. A fixed-size box resolving its own style
        /// counts there, and which retained slot answers it first can follow
        /// hash order; it never walks children, so that count is bounded, not
        /// exact.
        fn structural(self) -> Self {
            Self {
                measured: 0,
                ..self
            }
        }

        /// Everything but `result_children`. When what ends the page changes
        /// its extent, the page republishes its own result, which holds a
        /// placement for every group: that one copy follows the page's size,
        /// everything else is constant.
        fn without_page_result(self) -> Self {
            Self {
                result_children: 0,
                ..self
            }
        }

        /// The frontier a pass built and walked.
        fn frontier(self) -> (usize, usize, usize, usize, usize) {
            (
                self.seeds,
                self.frontier_measure,
                self.frontier_placement,
                self.contexts,
                self.edges,
            )
        }
    }

    /// Boxes `id` of `order` names: each one's local box.
    fn local_boxes(world: &UiWorld, order: &[StableNodeId]) -> Vec<Option<LayoutBox>> {
        order.iter().map(|id| world.local_layout_box(*id)).collect()
    }

    /// Of the boxes that `moved` on the page, those that kept their local box
    /// (`before` against now): a box above took them along, and nothing visits
    /// them for it -- not layout, and not what reads it.
    fn carried(
        world: &UiWorld,
        order: &[StableNodeId],
        before: &[Option<LayoutBox>],
        moved: &[bool],
    ) -> usize {
        order
            .iter()
            .zip(before)
            .zip(moved)
            .filter(|((id, before), moved)| **moved && world.local_layout_box(**id) == **before)
            .count()
    }

    /// One case run on a page: what the frame cost and how many boxes it moved.
    struct Case {
        cost: Cost,
        stats: LayoutFrontierStats,
        moved: usize,
    }

    fn run_case(page: &mut Page, scenario: Scenario, position: Position, seeds: u64) -> Case {
        let groups = page.groups_at(position, seeds);
        let order = page.world().document_order(page.document);
        let before = page.boxes();
        let before_local = local_boxes(page.world(), &order);
        page.edit(scenario, &groups);
        let counters = page.frame();
        let moved: Vec<bool> = before
            .iter()
            .zip(page.boxes())
            .map(|(old, new)| *old != new)
            .collect();
        let mut cost = Cost::from(counters);
        cost.translated = carried(page.world(), &order, &before_local, &moved);
        let moved = moved.iter().filter(|moved| **moved).count();
        Case {
            cost,
            stats: page.context.layout_frontier_stats(page.document),
            moved,
        }
    }

    /// The flow end whose growth moves nothing after it: the tail of a forward
    /// block axis, the head of vertical-rl's, which runs from the right edge
    /// while the page grows to the right.
    fn quiet_end(writing: Writing) -> Position {
        match writing {
            Writing::Ltr | Writing::Rtl => Position::Tail,
            Writing::VerticalRl => Position::Head,
        }
    }

    /// The gates one case meets, on a 1k page (`small`) and a 10k page (`large`).
    fn check_case(
        shape: Shape,
        scenario: Scenario,
        position: Position,
        edits: usize,
        small: &Case,
        large: &Case,
        label: &str,
    ) {
        // The document node, the page, the group, its wrappers and the body:
        // placement walks down them to reach an edit it does not move, once for
        // every edited group.
        let path = (shape.wrappers() as usize + 4) * edits;
        for case in [small, large] {
            let cost = case.cost;
            assert_eq!(
                (cost.local_fallbacks, cost.full_fallbacks),
                (0, 0),
                "{label}: {cost:?}"
            );
            if scenario == Scenario::PaintOnly {
                assert_eq!(cost, Cost::default(), "{label}: paint reached layout");
                assert_eq!(case.moved, 0, "{label}");
                continue;
            }
            // Placement pays for the boxes that moved, placed or taken along by
            // a container that only moved, and for the way down to the edit, and
            // for nothing else.
            let reached = cost.placed + cost.translated;
            assert!(
                reached >= case.moved,
                "{label}: {} boxes moved, {} placed, {} translated",
                case.moved,
                cost.placed,
                cost.translated
            );
            assert!(
                reached <= case.moved + cost.frontier_placement + path,
                "{label}: {} boxes moved, {} placed, {} translated: {cost:?}",
                case.moved,
                cost.placed,
                cost.translated
            );
        }
        if scenario == Scenario::PaintOnly {
            return;
        }
        let (small_cost, large_cost) = (small.cost, large.cost);
        if scenario == Scenario::Sequential {
            // Moving later siblings is real output and grows with them. What
            // was built, walked and measured to find them does not.
            assert_eq!(small_cost.frontier(), large_cost.frontier(), "{label}");
            assert_eq!(
                small_cost.full_subtrees, large_cost.full_subtrees,
                "{label}"
            );
            // Only the edited containers replay the part after their change;
            // the siblings that move are placed, not replayed.
            assert_eq!(
                small_cost.suffixes_replayed, large_cost.suffixes_replayed,
                "{label}"
            );
            // Boxes are in page coordinates, so every box in a group that moves
            // is written again -- inside a group that only moved, by translation
            // rather than placement (`translate_moved_children`); the group takes
            // its plans along, and only the edited containers miss or record
            // theirs again: the same number at every size, and never more than
            // boxes moved. (A context that records no plan, the inline one,
            // still walks its children when it moves.)
            assert_eq!(
                (small_cost.plan_misses, small_cost.plan_rebuilds),
                (large_cost.plan_misses, large_cost.plan_rebuilds),
                "{label}"
            );
            for case in [small, large] {
                assert!(
                    case.cost.plan_rebuilds <= case.moved,
                    "{label}: {} plans rebuilt, {} boxes moved",
                    case.cost.plan_rebuilds,
                    case.moved
                );
            }
            if position == quiet_end(shape.writing) {
                // The page's own placement of the edited group did change.
                assert_eq!(small.moved, large.moved, "{label}");
                assert_eq!(
                    small_cost.structural().without_page_result(),
                    large_cost.structural().without_page_result(),
                    "{label}"
                );
            }
        } else {
            assert_eq!(small.moved, large.moved, "{label}");
            assert_eq!(small_cost.structural(), large_cost.structural(), "{label}");
        }
        assert!(
            large_cost.measured <= 2 * small_cost.measured + 8,
            "{label}: {} nodes measured at 1k, {} at 10k",
            small_cost.measured,
            large_cost.measured
        );
    }

    fn matrix(writing: Writing) {
        for depth in [Depth::Shallow, Depth::Nested] {
            let shape = Shape { depth, writing };
            let mut small = Page::new(shape, 1_000, Guard::EveryPass);
            let mut large = Page::new(shape, 10_000, Guard::ColdOnly);
            assert!(large.world().len() >= 9_900);
            for scenario in Scenario::ALL {
                for position in Position::ALL {
                    for seeds in [1, 8] {
                        let label =
                            format!("{writing:?} {depth:?} {scenario:?} {position:?} x{seeds}");
                        let small_case = run_case(&mut small, scenario, position, seeds);
                        let large_case = run_case(&mut large, scenario, position, seeds);
                        check_case(
                            shape,
                            scenario,
                            position,
                            seeds as usize,
                            &small_case,
                            &large_case,
                            &label,
                        );
                    }
                }
            }
            small.assert_matches_cold();
            large.assert_matches_cold();
        }
    }

    /// Every scenario at the head, middle and tail of 1k and 10k pages, shallow
    /// and eight levels deep, with one seed and with eight.
    #[test]
    fn issue259_matrix_ltr() {
        matrix(Writing::Ltr);
    }

    #[test]
    fn issue259_matrix_rtl() {
        matrix(Writing::Rtl);
    }

    #[test]
    fn issue259_matrix_vertical_rl() {
        matrix(Writing::VerticalRl);
    }

    const CONTAINED: [Scenario; 7] = [
        Scenario::FixedContent,
        Scenario::ParentConstraint,
        Scenario::Flex,
        Scenario::Grid,
        Scenario::Inline,
        Scenario::Language,
        Scenario::Typography,
    ];

    /// A hundred edits in a hundred groups build one union closure: every edge
    /// of it is walked at most twice, and nothing costs more than a hundred
    /// single edits.
    #[test]
    fn issue259_a_hundred_seeds_cost_their_union_closure() {
        let shape = Shape {
            depth: Depth::Nested,
            writing: Writing::Ltr,
        };
        let mut page = Page::new(shape, 10_000, Guard::ColdOnly);
        assert!(page.groups >= 100);
        for scenario in CONTAINED {
            let single = run_case(&mut page, scenario, Position::Head, 1);
            let batch = run_case(&mut page, scenario, Position::Middle, 100);
            let label = format!("{scenario:?}");
            assert_eq!(batch.cost.seeds, 100 * single.cost.seeds, "{label}");
            assert!(
                batch.cost.edges <= 2 * batch.stats.graph_edges,
                "{label}: {} edge walks over a {}-edge closure",
                batch.cost.edges,
                batch.stats.graph_edges
            );
            for (name, one, hundred) in [
                (
                    "frontier",
                    single.cost.frontier_placement,
                    batch.cost.frontier_placement,
                ),
                ("edges", single.cost.edges, batch.cost.edges),
                ("placed", single.cost.placed, batch.cost.placed),
                (
                    "full subtrees",
                    single.cost.full_subtrees,
                    batch.cost.full_subtrees,
                ),
                (
                    "scratch",
                    single.stats.scratch_entries,
                    batch.stats.scratch_entries,
                ),
            ] {
                assert!(
                    hundred <= 100 * one,
                    "{label}: {name} {hundred} for a hundred edits, {one} for one"
                );
            }
            assert_eq!(batch.cost.full_fallbacks, 0, "{label}");
        }
        page.assert_matches_cold();
    }

    /// Local edits, and a sequential edit at the quiet end, cost the same on a
    /// 100k page as on a 10k one, one seed or a hundred, shallow and eight
    /// levels deep, in each writing mode.
    ///
    /// The local edits go to the other end. A hundred groups in the middle of a
    /// 10k nested page (263 groups) would overlap the hundred at its quiet end,
    /// whose retained state they warm: the 10k page would then do less than a
    /// fresh one, which the 100k page, too large to overlap, does not.
    fn contained_edits_at_100k(writing: Writing) {
        for depth in [Depth::Shallow, Depth::Nested] {
            let shape = Shape { depth, writing };
            let mut large = Page::new(shape, 10_000, Guard::ColdOnly);
            let mut huge = Page::new(shape, 100_000, Guard::ColdOnly);
            assert!(huge.world().len() >= 99_000);
            for scenario in CONTAINED.into_iter().chain([Scenario::Sequential]) {
                let position = match (scenario == Scenario::Sequential, quiet_end(writing)) {
                    (true, quiet) => quiet,
                    (false, Position::Head) => Position::Tail,
                    (false, _) => Position::Head,
                };
                for seeds in [1, 100] {
                    let label = format!("{writing:?} {depth:?} {scenario:?} x{seeds}");
                    let large_case = run_case(&mut large, scenario, position, seeds);
                    let huge_case = run_case(&mut huge, scenario, position, seeds);
                    assert_eq!(large_case.moved, huge_case.moved, "{label}");
                    let compared = |cost: Cost| {
                        if scenario == Scenario::Sequential {
                            cost.structural().without_page_result()
                        } else {
                            cost.structural()
                        }
                    };
                    assert_eq!(
                        compared(large_case.cost),
                        compared(huge_case.cost),
                        "{label}"
                    );
                    assert!(
                        huge_case.cost.measured <= 2 * large_case.cost.measured + 8,
                        "{label}: {} nodes measured at 10k, {} at 100k",
                        large_case.cost.measured,
                        huge_case.cost.measured
                    );
                    assert_eq!(
                        large_case.stats.scratch_entries, huge_case.stats.scratch_entries,
                        "{label}: scratch follows the closure, not the document"
                    );
                }
            }
            large.assert_matches_cold();
        }
    }

    #[test]
    #[ignore = "100k dense scale gate; run with --release -- --ignored"]
    fn issue259_contained_edits_cost_the_same_at_100k_ltr() {
        contained_edits_at_100k(Writing::Ltr);
    }

    #[test]
    #[ignore = "100k dense scale gate; run with --release -- --ignored"]
    fn issue259_contained_edits_cost_the_same_at_100k_rtl() {
        contained_edits_at_100k(Writing::Rtl);
    }

    #[test]
    #[ignore = "100k dense scale gate; run with --release -- --ignored"]
    fn issue259_contained_edits_cost_the_same_at_100k_vertical_rl() {
        contained_edits_at_100k(Writing::VerticalRl);
    }

    /// Ten thousand edits on a 100k page. The retained cache stays what the
    /// tree gives it: the same boxes and plans, no full-layout copy, intrinsic
    /// facts within their per-node budget. No frame's scratch outgrows one
    /// edit's closure. The page then matches a cold layout.
    #[test]
    #[ignore = "100k dense scale gate; run with --release -- --ignored"]
    fn issue259_ten_thousand_edits_on_100k_nodes_keep_the_retained_cache_flat() {
        use crate::layout_engine::RetainedLayoutFootprint;
        let mut page = Page::new(Shape::SHALLOW_LTR, 100_000, Guard::ColdOnly);
        let nodes = page.world().len();
        assert!(nodes >= 99_000);
        let kinds: Vec<Scenario> = CONTAINED
            .into_iter()
            .chain([Scenario::PaintOnly, Scenario::Sequential])
            .collect();
        let tail = page.groups - 1;
        // Each edit kind on and off once, so every plan the soak touches exists.
        for &scenario in &kinds {
            for _ in 0..2 {
                page.edit(scenario, &[tail]);
                page.frame();
            }
        }
        let settled = page.context.retained_layout_footprint(page.document);
        assert_eq!(
            settled.full_snapshots, 0,
            "the product frame keeps no full layout"
        );
        // What the tree gives the cache. Intrinsic facts and the last frontier
        // are held to their budgets instead.
        let flat = |footprint: RetainedLayoutFootprint| RetainedLayoutFootprint {
            intrinsics: 0,
            intrinsic_metrics: 0,
            last_frontier: 0,
            ..footprint
        };

        let mut group = 1u64;
        let mut peak_scratch = 0usize;
        for edit in 0..10_000usize {
            let scenario = kinds[edit % kinds.len()];
            // A sequential edit at the quiet end moves a constant; elsewhere it
            // would be the legitimate O(N) the matrix covers, not a soak frame.
            let target = if scenario == Scenario::Sequential {
                tail
            } else {
                group = (group * 7_919 + 13) % page.groups;
                group
            };
            page.edit(scenario, &[target]);
            let counters = page.frame();
            assert_eq!(counters.layout_full_document_fallbacks, 0, "edit {edit}");
            peak_scratch = peak_scratch.max(counters.layout_scratch_entries);
            if edit % 1_000 == 999 {
                let footprint = page.context.retained_layout_footprint(page.document);
                assert_eq!(flat(footprint), flat(settled), "after {} edits", edit + 1);
                assert!(footprint.intrinsics <= 2 * nodes, "{footprint:?}");
                assert!(footprint.intrinsic_metrics <= 4 * nodes, "{footprint:?}");
                assert!(footprint.last_frontier <= 64, "{footprint:?}");
            }
        }
        assert!(
            peak_scratch <= 64,
            "a frame of one edit held {peak_scratch} scratch entries"
        );
        page.assert_matches_cold();
    }

    /// The timing half of the contract, for the benchmark machine: p50, p95 and
    /// p99 of a local edit's frame at 1k, 10k and 100k nodes, and the p95 growth
    /// from 1k to 100k against the 1.75x target. PR CI gates on the counters
    /// above; timings are recorded where the machine is fixed:
    ///
    /// `cargo test --release -p nana-ui-runtime --lib issue259_local_frame_time -- --ignored --nocapture`
    #[test]
    #[ignore = "fixed-machine timing: run with --release -- --ignored"]
    fn issue259_local_frame_time_scales_flat_to_100k() {
        use std::time::{Duration, Instant};
        let shape = Shape::SHALLOW_LTR;
        let mut p95 = Vec::new();
        for nodes in [1_000u64, 10_000, 100_000] {
            let mut page = Page::new(shape, nodes, Guard::ColdOnly);
            let middle = page.groups / 2;
            for scenario in CONTAINED {
                for _ in 0..4 {
                    page.edit(scenario, &[middle]);
                    page.frame();
                }
            }
            let mut samples: Vec<Duration> = Vec::with_capacity(500);
            for edit in 0..500u64 {
                let scenario = CONTAINED[edit as usize % CONTAINED.len()];
                page.edit(scenario, &[(edit * 7_919 + 13) % page.groups]);
                let started = Instant::now();
                page.frame();
                samples.push(started.elapsed());
            }
            samples.sort_unstable();
            let at =
                |quantile: f64| samples[((samples.len() - 1) as f64 * quantile).round() as usize];
            eprintln!(
                "{nodes} nodes: p50 {:?}, p95 {:?}, p99 {:?}",
                at(0.50),
                at(0.95),
                at(0.99)
            );
            p95.push(at(0.95));
        }
        let growth = p95[2].as_secs_f64() / p95[0].as_secs_f64();
        eprintln!("p95 at 100k / p95 at 1k: {growth:.2}");
        assert!(
            growth <= 1.75,
            "a local edit's p95 frame grew {growth:.2}x from 1k to 100k nodes"
        );
    }

    /// The widths of a window resize storm: out by 120 pixels and back to the
    /// page's own viewport, one a frame.
    fn storm_widths() -> impl Iterator<Item = f32> {
        (1..=240).map(|step: u32| {
            let out = if step <= 120 { step } else { 240 - step };
            viewport().width + out as f32
        })
    }

    /// A window resize storm (Issue #264): 240 viewport widths. The page is a
    /// fixed column, so each resize reaches the document root and no group: the
    /// cost is the same at 1k, 10k and 100k nodes, and nothing falls back.
    #[test]
    fn issue259_a_viewport_resize_storm_stays_at_the_root() {
        let shape = Shape::SHALLOW_LTR;
        let sizes: &[u64] = if cfg!(debug_assertions) {
            &[1_000, 10_000]
        } else {
            &[1_000, 10_000, 100_000]
        };
        let mut pages: Vec<Page> = sizes
            .iter()
            .map(|&nodes| {
                let guard = if nodes == 1_000 {
                    Guard::EveryPass
                } else {
                    Guard::ColdOnly
                };
                Page::new(shape, nodes, guard)
            })
            .collect();
        assert!(
            pages.last().unwrap().world().len() >= (*sizes.last().unwrap() as usize * 99 / 100)
        );
        for width in storm_widths() {
            let viewport = LayoutViewport::new(width, viewport().height);
            let costs: Vec<Cost> = pages
                .iter_mut()
                .map(|page| Cost::from(page.resize(viewport)))
                .collect();
            assert_eq!(
                (costs[0].local_fallbacks, costs[0].full_fallbacks),
                (0, 0),
                "{width}"
            );
            for pair in costs.windows(2) {
                assert_eq!(pair[0], pair[1], "{width}");
            }
        }
        for page in &mut pages {
            page.assert_matches_cold();
        }
    }

    /// The same storm on a vertical-rl page, whose block axis runs from the
    /// right edge: each pass matches a full layout.
    #[test]
    fn issue259_a_vertical_rl_resize_storm_matches_a_full_layout() {
        let shape = Shape {
            depth: Depth::Nested,
            writing: Writing::VerticalRl,
        };
        let mut page = Page::new(shape, 1_000, Guard::EveryPass);
        for width in storm_widths().step_by(9).chain([viewport().width]) {
            let counters = page.resize(LayoutViewport::new(width, viewport().height));
            assert_eq!(counters.layout_full_document_fallbacks, 0, "{width}");
        }
        page.assert_matches_cold();
    }

    /// Theme workloads (Issue #261): a Light/Dark switch is a palette, and a
    /// metrics install reaches only boxes that declare design intent, which
    /// these pages have none of. Neither lays anything out, at any size.
    #[test]
    fn issue259_theme_switches_lay_nothing_out_at_any_size() {
        let sizes: &[u64] = if cfg!(debug_assertions) {
            &[1_000, 10_000]
        } else {
            &[1_000, 10_000, 100_000]
        };
        for &nodes in sizes {
            let guard = if nodes == 1_000 {
                Guard::EveryPass
            } else {
                Guard::ColdOnly
            };
            let mut page = Page::new(Shape::SHALLOW_LTR, nodes, guard);
            for mode in [
                nana_ui_core::ThemeAppearance::Light,
                nana_ui_core::ThemeAppearance::Dark,
            ] {
                page.context.set_preset_theme(mode).unwrap();
                let counters = page.frame();
                assert_eq!(Cost::from(counters), Cost::default(), "{nodes} {mode:?}");
                assert_eq!(counters.theme_palette_layout_invalidations, 0);
            }
            crate::world::tests::incremental_work::theme_invalidation::widen_control_padding(
                &mut page.context,
                4.0,
            );
            let counters = page.frame();
            assert_eq!(Cost::from(counters), Cost::default(), "{nodes} metrics");
            assert_eq!(counters.theme_to_layout_seeds, 0);
            assert_eq!(counters.theme_metric_dependents_invalidated, 0);
        }
    }

    /// Replaced content workloads (Issue #263), appended to the end of 1k, 10k
    /// and 100k pages: a video's frames lay nothing out, an image's natural size
    /// lays out its box, and a shared image reaches the twenty nodes showing it.
    /// Each costs the same at every size.
    #[test]
    fn issue259_replaced_content_costs_the_same_at_any_size() {
        use crate::world::tests::replaced_media::{image_layout, url};
        use crate::{CustomRenderNode, HOST_TEXTURE_RENDERER, ReplacedMetadata};
        let image = |name: &str, side: Option<f32>| styled(image_layout(name, side, side));
        let mut costs: Vec<Vec<Cost>> = Vec::new();
        let sizes: &[u64] = if cfg!(debug_assertions) {
            &[1_000, 10_000]
        } else {
            &[1_000, 10_000, 100_000]
        };
        for &nodes in sizes {
            let guard = if nodes == 1_000 {
                Guard::EveryPass
            } else {
                Guard::ColdOnly
            };
            let mut page = Page::new(Shape::SHALLOW_LTR, nodes, guard);
            let base = 10_000_000u64;
            let media = node(base);
            let video = node(base + 1);
            let mut tree = Tree::new(page.document);
            tree.element(node(2), media, "media", column());
            tree.element(
                media,
                video,
                "video",
                styled(LayoutStyle {
                    width: Some(LengthSpec::Px(160.0)),
                    height: Some(LengthSpec::Px(90.0)),
                    ..LayoutStyle::default()
                }),
            );
            tree.queue.set_custom_render(
                video,
                Some(CustomRenderNode::new(HOST_TEXTURE_RENDERER, "video:1", 0)),
            );
            tree.element(media, node(base + 2), "img", image("hero", None));
            for index in 0..20u64 {
                let side = (index % 2 == 0).then_some(40.0);
                tree.element(media, node(base + 10 + index), "img", image("shared", side));
            }
            page.context.commit_mutations(tree.queue).unwrap();
            page.frame();
            let mut row = Vec::new();
            for frame in 1..=30u64 {
                let mut queue = MutationQueue::new();
                queue.set_custom_render(
                    video,
                    Some(CustomRenderNode::new(
                        HOST_TEXTURE_RENDERER,
                        "video:1",
                        frame,
                    )),
                );
                page.context.commit_mutations(queue).unwrap();
                let counters = page.frame();
                assert_eq!(
                    Cost::from(counters),
                    Cost::default(),
                    "{nodes}: frame {frame}"
                );
            }
            for (resource, size) in [(url("hero"), (320.0, 180.0)), (url("shared"), (24.0, 16.0))] {
                let mut queue = MutationQueue::new();
                queue.set_replaced_metadata(resource, Some(ReplacedMetadata::new(size.0, size.1)));
                page.context.commit_mutations(queue).unwrap();
                let counters = page.frame();
                assert_eq!(counters.layout_full_document_fallbacks, 0);
                row.push(Cost::from(counters).without_page_result());
            }
            costs.push(row);
        }
        for pair in costs.windows(2) {
            assert_eq!(pair[0], pair[1]);
        }
    }

    /// Issue #262's feed of 100k and of 1M rows: a visible row growing by 8 px
    /// costs the same at both sizes -- the list's window, never its rows. Only
    /// the row index's depth, O(log C) entries over C chunks, follows the
    /// collection.
    #[test]
    fn issue259_a_virtual_row_growing_costs_the_same_at_100k_and_1m() {
        use crate::world::tests::incremental_work::virtual_scroll::{Feed, Pass, grow};
        let mut costs = Vec::new();
        for rows in [100_000, 1_000_000] {
            let mut feed = Feed::new(rows);
            feed.scroll_to_row(rows / 2);
            let growth = grow(&mut feed, 10);
            assert!(
                growth.pass.index_updates <= 512 + 32,
                "{rows}: {:?}",
                growth.pass
            );
            costs.push((
                Pass {
                    index_updates: 0,
                    ..growth.pass
                },
                growth.lookups,
                Cost::from(growth.laid),
                Cost::from(growth.placed),
            ));
        }
        assert_eq!(costs[0], costs[1]);
    }

    /// Issue #265's panel of a hundred responsive rows and its eight nested
    /// containers, at the end of 1k, 10k and 100k node pages: a resize within
    /// the rules' buckets, one across a breakpoint and one back, and a nested
    /// resize across every level's, cost the same at every size.
    #[test]
    fn issue259_responsive_rules_cost_the_same_at_any_size() {
        use crate::world::tests::dynamic_layout::responsive_queries::{Panel, Queries};
        let mut costs: Vec<Vec<(Queries, Cost)>> = Vec::new();
        let sizes: &[u64] = if cfg!(debug_assertions) {
            &[1_000, 10_000]
        } else {
            &[1_000, 10_000, 100_000]
        };
        for &nodes in sizes {
            let guarded = nodes == 1_000;
            let mut row = Vec::new();
            let mut panel = Panel::new(nodes, 800.0, guarded);
            for width in [790.0, 480.0, 790.0] {
                let counters = panel.resize(width);
                assert_eq!(counters.layout_full_document_fallbacks, 0);
                row.push((
                    Queries::from(counters),
                    Cost::from(counters).without_page_result(),
                ));
            }
            let mut nest = Panel::nest(800.0, nodes, guarded);
            for width in [300.0, 800.0] {
                let counters = nest.resize(width);
                assert_eq!(counters.layout_full_document_fallbacks, 0);
                row.push((
                    Queries::from(counters),
                    Cost::from(counters).without_page_result(),
                ));
            }
            costs.push(row);
        }
        for pair in costs.windows(2) {
            assert_eq!(pair[0], pair[1]);
        }
    }

    /// Issues #268 and #269: forty localized labels in a bar of a fixed height
    /// at the end of 1k, 10k and 100k node windows switch locale at the same
    /// cost: left to right to left to right (en-US to zh-CN), and an equal
    /// translation (en-US to en-GB with the language named) whose unchanged
    /// strings do no text work. The rest of the window is never visited.
    #[test]
    fn issue259_a_locale_switch_costs_the_same_at_any_size() {
        use crate::Locale;
        use crate::world::tests::localization::catalog_switching::{App, cost, tag};
        let mut costs = Vec::new();
        let sizes: &[u64] = if cfg!(debug_assertions) {
            &[1_000, 10_000]
        } else {
            &[1_000, 10_000, 100_000]
        };
        for &nodes in sizes {
            let mut app = App::labelled(nodes, 40, nodes == 1_000);
            assert_eq!(app.windows[0].localized.len(), 40, "{nodes}");
            let mut row = Vec::new();
            let mut measure = |app: &mut App, change: &dyn Fn(&mut App)| {
                let switch = cost(app, change);
                let counters = app.frame(0);
                assert_eq!(counters.layout_full_document_fallbacks, 0, "{nodes}");
                row.push((
                    switch.direction_changed_scopes,
                    switch.scope_dependents_notified,
                    switch.nodes_resolved,
                    switch.resolved_content_changed,
                    switch.resolved_content_unchanged,
                    switch.literal_nodes,
                    counters.text_shaped,
                    Cost::from(counters).without_page_result(),
                ));
            };
            let pinned = |messages: &str| Locale::parse(messages).unwrap().with_language(tag("en"));
            for locale in [
                Locale::parse("zh-cn"),
                Some(pinned("en-us")),
                Some(pinned("en-gb")),
            ] {
                measure(&mut app, &|app| {
                    app.context.set_default_locale(locale.clone())
                });
            }
            // The bar becomes a scope of its own, then turns right to left:
            // the bar lays out again, and nothing outside it.
            let bar = node(1_000_000 + 999_000);
            for locale in [Locale::parse("en-us"), Locale::parse("ar")] {
                measure(&mut app, &|app| {
                    let mut queue = MutationQueue::new();
                    queue.set_locale(bar, locale.clone());
                    app.context.commit_mutations(queue).unwrap();
                });
            }
            costs.push(row);
        }
        for pair in costs.windows(2) {
            assert_eq!(pair[0], pair[1]);
        }
    }

    /// Issue #269: a virtual list of localized rows switches locale at the same
    /// cost at 100k and at 1M rows: its mounted rows resolve, no logical row is
    /// looked up.
    #[test]
    fn issue259_a_localized_virtual_list_switches_at_the_same_cost_at_any_size() {
        use crate::Locale;
        use crate::world::tests::localization::virtual_feed::LocalizedFeed;
        let mut costs = Vec::new();
        for rows in [100_000, 1_000_000] {
            let mut feed = LocalizedFeed::new(rows);
            let mounted = feed.items.mounted_keys().len();
            let (switch, scanned, frame) = feed.switch(Locale::parse("zh-cn"));
            assert_eq!(scanned, 0);
            assert!(
                switch.virtual_rows_resolved > 0 && switch.virtual_rows_resolved <= mounted,
                "{rows}: {} virtual rows resolved, {mounted} mounted",
                switch.virtual_rows_resolved
            );
            costs.push((
                switch.nodes_resolved,
                switch.virtual_rows_resolved,
                frame.text_shaped,
                Cost::from(frame),
            ));
        }
        assert_eq!(costs[0], costs[1]);
    }

    /// Issue #270: in 10k and 100k node windows ending in forty localized labels
    /// and a counter, the counter's count changing, a catalog update of one
    /// message, and a switch to German, whose formatters are built then, cost
    /// the same: formatting reaches the nodes whose output it is.
    #[test]
    fn issue259_formatting_costs_the_same_at_any_size() {
        use crate::world::tests::localization::catalog_switching::{App, cost};
        use crate::world::tests::localization::message_formatting::counting_catalog;
        use crate::{Locale, LocalizedText, MessageId};
        let mut costs = Vec::new();
        let sizes: &[u64] = if cfg!(debug_assertions) {
            &[1_000, 10_000]
        } else {
            &[10_000, 100_000]
        };
        for &nodes in sizes {
            let mut app = App::labelled(nodes, 40, false);
            app.context.set_message_catalog(Some(counting_catalog()));
            let counter = node(1_000_000 + 999_000 + 500);
            let mut queue = MutationQueue::new();
            queue.create(counter, app.windows[0].document, NodeKind::Text);
            queue.insert(node(1_000_000 + 999_000), counter, None);
            queue.set_localized_text(
                counter,
                Some(LocalizedText::new("files").arg("count", 1u32)),
            );
            app.context.commit_mutations(queue).unwrap();
            app.frame(0);
            let mut row = Vec::new();
            let mut measure = |app: &mut App, change: &dyn Fn(&mut App)| {
                let update = cost(app, change);
                let counters = app.frame(0);
                assert_eq!(counters.layout_full_document_fallbacks, 0);
                row.push((
                    update.format_requests,
                    update.nodes_resolved,
                    update.catalog_messages_invalidated,
                    update.message_patterns_compiled,
                    update.formatter_allocations,
                    update.formatted_output_changed,
                    counters.text_shaped,
                    Cost::from(counters).without_page_result(),
                ));
            };
            for count in [2u32, 3, 40] {
                measure(&mut app, &|app| {
                    let mut queue = MutationQueue::new();
                    queue.set_localized_text(
                        counter,
                        Some(LocalizedText::new("files").arg("count", count)),
                    );
                    app.context.commit_mutations(queue).unwrap();
                });
            }
            measure(&mut app, &|app| {
                let updated = Arc::new((*counting_catalog()).clone().with(
                    "en-us",
                    "item.7",
                    "Updated item of {count}",
                ));
                app.context
                    .compat_world_mut()
                    .update_message_catalog(updated, &[MessageId::new("item.7")]);
            });
            measure(&mut app, &|app| {
                app.context.set_default_locale(Locale::parse("de"))
            });
            costs.push(row);
        }
        assert_eq!(costs[0], costs[1]);
    }

    /// A label at the head of a page, in a column 1200 px wide that it does not
    /// fill, above 1k and 10k nodes of cards, takes a longer string that still
    /// fits the column. Its box was only as wide as the string it held, so the
    /// new one is not wrapped to that width: the frame lays out once, the label
    /// on one line at its new width, and nothing below it moves -- what the same
    /// edit costs a label that cannot wrap, and the same at both sizes.
    #[test]
    fn issue259_a_longer_label_moves_nothing_below_it() {
        use crate::world::reflow_oracle::{self, Builder, FILLER_GROUP_NODES};
        const SHORT: &str = "Short";
        const LONGER: &str = "A much longer label that still fits on one line of its column";
        let viewport = LayoutViewport::new(1200.0, 800.0);
        // The cards sit in a column of their own: the page holds two boxes at
        // any size.
        let build = |nodes: u64, wraps: bool, value: &str| {
            let document = DocumentId::new(1).unwrap();
            let (mut b, page) = Builder::page(document, 1, 1200.0);
            // A flex column: the label keeps the width of its string, which a
            // block column would stretch to 1200.
            let head = b.element(
                page,
                LayoutStyle {
                    display: Some(nana_ui_core::DisplaySpec::Flex),
                    ..reflow_oracle::column(Some(1200.0))
                },
            );
            let label = b.label(head, value);
            if !wraps {
                b.queue.set_style(
                    label,
                    styled(LayoutStyle {
                        white_space_nowrap: true,
                        ..LayoutStyle::default()
                    }),
                );
            }
            let body = b.element(page, reflow_oracle::column(None));
            b.filler(body, nodes / FILLER_GROUP_NODES);
            let mut context = AppContext::new();
            context.commit_mutations(b.queue).unwrap();
            let mut shaper = bundled_face_shaper();
            product_frame(&mut context, document, viewport, &mut shaper);
            (context, document, label, shaper)
        };
        let mut costs = Vec::new();
        for nodes in [1_000, 10_000] {
            let _unguarded = (nodes > 1_000).then(skip_layout_verify);
            let mut row = Vec::new();
            for wraps in [false, true] {
                let case = format!("{nodes} nodes, wraps {wraps}");
                let (mut context, document, label, mut shaper) = build(nodes, wraps, SHORT);
                let order = context.world().document_order(document);
                let boxes = |context: &AppContext| {
                    order
                        .iter()
                        .map(|id| context.world().layout_box(*id))
                        .collect::<Vec<_>>()
                };
                let before = boxes(&context);
                let passes = context.layout_invocations();
                let mut queue = MutationQueue::new();
                queue.set_text(
                    label,
                    TextContent {
                        value: LONGER.into(),
                    },
                );
                context.commit_mutations(queue).unwrap();
                let counters = product_frame(&mut context, document, viewport, &mut shaper);
                let moved: Vec<StableNodeId> = order
                    .iter()
                    .zip(before.iter().zip(boxes(&context)))
                    .filter(|(_, (old, new))| **old != *new)
                    .map(|(id, _)| *id)
                    .collect();
                assert_eq!(moved, [label], "{case}");
                // Nor did anything move and move back on the way.
                assert_eq!(counters.layout_origin_only_updates, 0, "{case}");
                assert_eq!(context.layout_invocations() - passes, 1, "{case}");
                let (mut cold, ..) = build(nodes, wraps, LONGER);
                assert_matches_cold(&mut context, &mut cold, document);
                row.push(Cost::from(counters).structural());
            }
            assert_eq!(row[0], row[1], "{nodes} nodes: wrapping costs extra");
            costs.push(row[1]);
        }
        assert_eq!(costs[0], costs[1]);
    }

    /// A label at the head of a page, in a column, and the page's groups of
    /// cards directly after that column: the flow the column heads is as long
    /// as the page, 1k and 10k nodes. The label takes a longer string. One that
    /// fits its line, wrapping or not, leaves the column as tall as it was: the
    /// frame builds, walks and places the same at both sizes, and only the label
    /// moves. One that wraps in a narrow column grows it: every group after it
    /// moves down once -- real output, which grows with the page -- while what
    /// was built, walked and measured to find them does not. Each group takes
    /// its cards along at their offsets in it: the children walked, the plans
    /// missed and the sizes looked up do not grow with the page either.
    #[test]
    fn issue259_a_label_costs_its_column_not_the_groups_after_it() {
        use crate::world::reflow_oracle::{self, Builder, FILLER_GROUP_NODES};
        const FITS: &str = "A much longer label that still fits on one line of the column";
        const WRAPS: &str = "A much longer label that wraps in a column three hundred pixels wide";
        let viewport = LayoutViewport::new(1200.0, 800.0);
        let build = |nodes: u64, width: f32, wraps: bool, value: &str| {
            let document = DocumentId::new(1).unwrap();
            let (mut b, page) = Builder::page(document, 1, 1200.0);
            // A flex column: the label keeps the width of its string, which a
            // block column would stretch to the column's.
            let head = b.element(
                page,
                LayoutStyle {
                    display: Some(nana_ui_core::DisplaySpec::Flex),
                    ..reflow_oracle::column(Some(width))
                },
            );
            let label = b.label(head, value);
            if !wraps {
                b.queue.set_style(
                    label,
                    styled(LayoutStyle {
                        white_space_nowrap: true,
                        ..LayoutStyle::default()
                    }),
                );
            }
            b.filler(page, nodes / FILLER_GROUP_NODES);
            let mut context = AppContext::new();
            context.commit_mutations(b.queue).unwrap();
            let mut shaper = bundled_face_shaper();
            product_frame(&mut context, document, viewport, &mut shaper);
            (context, document, label, shaper)
        };
        // The case, the column's width, whether the label wraps, the string it
        // takes, and whether that grows the column.
        for (case, width, wraps, value, grows) in [
            ("fits, cannot wrap", 1200.0, false, FITS, false),
            ("fits, wraps", 1200.0, true, FITS, false),
            ("grows", 300.0, true, WRAPS, true),
        ] {
            let mut found = Vec::new();
            let mut costs = Vec::new();
            let mut lookups = Vec::new();
            for nodes in [1_000, 10_000] {
                let _unguarded = (nodes > 1_000).then(skip_layout_verify);
                let label_case = format!("{case}, {nodes} nodes");
                let (mut context, document, label, mut shaper) =
                    build(nodes, width, wraps, "Short");
                let order = context.world().document_order(document);
                // The document, the page, the column and the label; the groups
                // are everything after them.
                let groups = order.iter().position(|id| *id == label).unwrap() + 1;
                let boxes = |context: &AppContext| {
                    order
                        .iter()
                        .map(|id| context.world().layout_box(*id))
                        .collect::<Vec<_>>()
                };
                let before = boxes(&context);
                let before_local = local_boxes(context.world(), &order);
                let passes = context.layout_invocations();
                let mut queue = MutationQueue::new();
                queue.set_text(
                    label,
                    TextContent {
                        value: value.into(),
                    },
                );
                context.commit_mutations(queue).unwrap();
                let counters = product_frame(&mut context, document, viewport, &mut shaper);
                let passes = context.layout_invocations() - passes;
                let moved: Vec<bool> = before
                    .iter()
                    .zip(boxes(&context))
                    .map(|(old, new)| *old != new)
                    .collect();
                let mut cost = Cost::from(counters);
                cost.translated = carried(context.world(), &order, &before_local, &moved);
                if grows {
                    // Every group moved, and only once.
                    assert!(moved[groups..].iter().all(|moved| *moved), "{label_case}");
                    // A group placed only by moving takes what it holds along
                    // without placing it (`translate_moved_children`).
                    assert_eq!(
                        cost.origin_only + cost.translated,
                        order.len() - groups,
                        "{label_case}"
                    );
                    let moved = moved.iter().filter(|moved| **moved).count();
                    let reached = cost.placed + cost.translated;
                    assert!(
                        reached >= moved && reached <= moved + cost.frontier_placement,
                        "{label_case}: {moved} boxes moved, {} placed: {cost:?}",
                        cost.placed
                    );
                } else {
                    let moved: Vec<StableNodeId> = order
                        .iter()
                        .zip(&moved)
                        .filter(|(_, moved)| **moved)
                        .map(|(id, _)| *id)
                        .collect();
                    assert_eq!(moved, [label], "{label_case}");
                    assert_eq!(cost.origin_only, 0, "{label_case}");
                    assert_eq!(passes, 1, "{label_case}");
                }
                let (mut cold, ..) = build(nodes, width, wraps, value);
                assert_matches_cold(&mut context, &mut cold, document);
                found.push((
                    passes,
                    cost.frontier(),
                    cost.full_subtrees,
                    // A group that moved took its cards along: none of them was
                    // walked to place it, nor its plan missed.
                    (cost.children_walked, cost.plan_misses, cost.plan_rebuilds),
                    context.layout_frontier_stats(document),
                ));
                costs.push(cost);
                lookups.push(counters.layout_measure_cache_hits);
            }
            // What was built, walked and measured to find what moved, and walked
            // to move it.
            assert_eq!(found[0], found[1], "{case}");
            assert!(
                lookups[1] <= 2 * lookups[0] + 8,
                "{case}: {} sizes looked up at 1k, {} at 10k",
                lookups[0],
                lookups[1]
            );
            if !grows {
                assert_eq!(costs[0].structural(), costs[1].structural(), "{case}");
            }
            assert!(
                costs[1].measured <= 2 * costs[0].measured + 8,
                "{case}: {} nodes measured at 1k, {} at 10k",
                costs[0].measured,
                costs[1].measured
            );
        }
    }

    /// A column at the head of a page, 300 px wide, and the page's groups of
    /// cards directly after it, 1k and 10k nodes. A fixed box arrives in the
    /// column, or in the page before it, and leaves again. A column as tall as
    /// what it holds grows and shrinks back, as does the page, and every box
    /// after the new one moves once each way -- real output -- while what was
    /// built, walked and measured to find them does not grow: none of them is
    /// on the frontier. Nor does what moves them: each group takes its cards
    /// along at their offsets in it, so the children walked, the plans missed
    /// and the sizes looked up stay the same. A column of a fixed height keeps
    /// its box, and the frame costs the same at both sizes.
    #[test]
    fn issue259_a_child_list_edit_costs_its_parent_not_the_groups_after_it() {
        use crate::world::reflow_oracle::{self, Builder, FILLER_GROUP_NODES};
        let viewport = LayoutViewport::new(1200.0, 800.0);
        // After every id the builder hands out.
        let arriving = node(1_000_000);
        let arrive = |queue: &mut MutationQueue, document, (parent, before)| {
            queue.create(arriving, document, NodeKind::Element { tag: "div".into() });
            queue.insert(parent, arriving, before);
            queue.set_style(arriving, styled(reflow_oracle::fixed(100.0, 20.0)));
        };
        let build = |nodes: u64, height: Option<f32>, in_page: bool, inserted: bool| {
            let document = DocumentId::new(1).unwrap();
            let (mut b, page) = Builder::page(document, 1, 1200.0);
            let head = b.element(
                page,
                LayoutStyle {
                    height: height.map(LengthSpec::Px),
                    ..reflow_oracle::column(Some(300.0))
                },
            );
            let label = b.label(head, "Short");
            b.filler(page, nodes / FILLER_GROUP_NODES);
            // Where the box arrives, and the last box before the ones it moves.
            let (at, still) = if in_page {
                ((page, Some(head)), page)
            } else {
                ((head, None), label)
            };
            if inserted {
                arrive(&mut b.queue, document, at);
            }
            let mut context = AppContext::new();
            context.commit_mutations(b.queue).unwrap();
            let mut shaper = bundled_face_shaper();
            product_frame(&mut context, document, viewport, &mut shaper);
            (context, document, at, still, shaper)
        };
        for (height, in_page) in [(None, false), (Some(60.0), false), (None, true)] {
            let case = format!("height {height:?}, in page {in_page}");
            let grows = height.is_none();
            let mut found = Vec::new();
            let mut costs = Vec::new();
            let mut lookups = Vec::new();
            for nodes in [1_000, 10_000] {
                let _unguarded = (nodes > 1_000).then(skip_layout_verify);
                let (mut context, document, at, still, mut shaper) =
                    build(nodes, height, in_page, false);
                let order = context.world().document_order(document);
                let moving = order.iter().position(|id| *id == still).unwrap() + 1;
                let boxes = |context: &AppContext| {
                    order
                        .iter()
                        .map(|id| context.world().layout_box(*id))
                        .collect::<Vec<_>>()
                };
                for inserting in [true, false] {
                    let case = format!("{case}, inserting {inserting}, {nodes} nodes");
                    let before = boxes(&context);
                    let before_local = local_boxes(context.world(), &order);
                    let passes = context.layout_invocations();
                    let mut queue = MutationQueue::new();
                    if inserting {
                        arrive(&mut queue, document, at);
                    } else {
                        queue.despawn_subtree(arriving);
                    }
                    context.commit_mutations(queue).unwrap();
                    let counters = product_frame(&mut context, document, viewport, &mut shaper);
                    assert_eq!(context.layout_invocations() - passes, 1, "{case}");
                    let moved: Vec<bool> = before
                        .iter()
                        .zip(boxes(&context))
                        .map(|(old, new)| *old != new)
                        .collect();
                    // Every box after the new one moved, and only once, or none
                    // did; and none was on the frontier.
                    assert!(
                        moved[moving..].iter().all(|moved| *moved == grows),
                        "{case}"
                    );
                    assert!(
                        !order[moving..]
                            .iter()
                            .any(|id| reflow_oracle::admitted(&context, *id)),
                        "{case}"
                    );
                    let mut cost = Cost::from(counters);
                    cost.translated = carried(context.world(), &order, &before_local, &moved);
                    let shifted = if grows { order.len() - moving } else { 0 };
                    assert_eq!(cost.origin_only + cost.translated, shifted, "{case}");
                    let moved = moved.iter().filter(|moved| **moved).count();
                    let reached = cost.placed + cost.translated;
                    assert!(
                        reached >= moved && reached <= moved + cost.frontier_placement,
                        "{case}: {moved} boxes moved, {} placed: {cost:?}",
                        cost.placed
                    );
                    let (mut cold, ..) = build(nodes, height, in_page, inserting);
                    assert_matches_cold(&mut context, &mut cold, document);
                    found.push((
                        cost.frontier(),
                        cost.full_subtrees,
                        // A group that moved took its cards along: none of them
                        // was walked to place it, nor its plan missed.
                        (cost.children_walked, cost.plan_misses, cost.plan_rebuilds),
                        context.layout_frontier_stats(document),
                    ));
                    costs.push(cost);
                    lookups.push(counters.layout_measure_cache_hits);
                }
            }
            // What was built, walked and measured to find them, and walked to
            // move them: the insertion and the removal, at 1k and at 10k.
            assert_eq!(found[..2], found[2..], "{case}");
            for ((small, large), (small_lookups, large_lookups)) in costs[..2]
                .iter()
                .zip(&costs[2..])
                .zip(lookups[..2].iter().zip(&lookups[2..]))
            {
                if !grows {
                    assert_eq!(small.structural(), large.structural(), "{case}");
                }
                assert!(
                    large.measured <= 2 * small.measured + 8,
                    "{case}: {} nodes measured at 1k, {} at 10k",
                    small.measured,
                    large.measured
                );
                assert!(
                    *large_lookups <= 2 * small_lookups + 8,
                    "{case}: {small_lookups} sizes looked up at 1k, {large_lookups} at 10k"
                );
            }
        }
    }

    /// The page's own child list, 1k and 10k nodes: the 300 px column at its
    /// head, its groups of cards, and a fixed 100x20 box at one end of it. The
    /// box leaves, in the first frame after the page was laid out from nothing,
    /// and arrives again. No subtree is walked to measure it, the page's
    /// included: the page's measure and placement take the edit from the plans
    /// they recorded, and the groups it kept are not measured again. At the tail
    /// nothing else moves, and the frame costs the same at both sizes but for
    /// the page's own result, which holds a placement for every group. At the
    /// head every box after the edit moves once -- real output -- and each group
    /// takes its cards along at their offsets in it: the children walked, the
    /// plans missed and the sizes looked up are the same at both sizes.
    #[test]
    fn issue259_a_child_list_edit_in_the_page_measures_the_edit_not_the_page() {
        use crate::world::reflow_oracle::{self, Builder, FILLER_GROUP_NODES};
        let viewport = LayoutViewport::new(1200.0, 800.0);
        // After every id the builder hands out: the box the page starts with,
        // and the one that arrives once it left.
        let (leaving, arriving) = (node(1_000_000), node(1_000_001));
        let place = |queue: &mut MutationQueue, document, page, before, edge| {
            queue.create(edge, document, NodeKind::Element { tag: "div".into() });
            queue.insert(page, edge, before);
            queue.set_style(edge, styled(reflow_oracle::fixed(100.0, 20.0)));
        };
        let build = |nodes: u64, head: bool, edge: Option<StableNodeId>| {
            let document = DocumentId::new(1).unwrap();
            let (mut b, page) = Builder::page(document, 1, 1200.0);
            let column = b.element(page, reflow_oracle::column(Some(300.0)));
            b.label(column, "Short");
            b.filler(page, nodes / FILLER_GROUP_NODES);
            // The box sits before the column, or after the last group.
            let before = head.then_some(column);
            if let Some(edge) = edge {
                place(&mut b.queue, document, page, before, edge);
            }
            let mut context = AppContext::new();
            context.commit_mutations(b.queue).unwrap();
            let mut shaper = bundled_face_shaper();
            product_frame(&mut context, document, viewport, &mut shaper);
            (context, document, page, before, shaper)
        };
        for head in [true, false] {
            let case = if head { "head" } else { "tail" };
            let mut found = Vec::new();
            let mut costs = Vec::new();
            let mut lookups = Vec::new();
            for nodes in [1_000, 10_000] {
                let _unguarded = (nodes > 1_000).then(skip_layout_verify);
                let (mut context, document, page, before, mut shaper) =
                    build(nodes, head, Some(leaving));
                // The nodes before and after the edit, the box aside: the
                // document, the page, and everything the page holds after it.
                let order: Vec<StableNodeId> = context
                    .world()
                    .document_order(document)
                    .into_iter()
                    .filter(|id| *id != leaving)
                    .collect();
                let after = order.iter().position(|id| *id == page).unwrap() + 1;
                let boxes = |context: &AppContext| {
                    order
                        .iter()
                        .map(|id| context.world().layout_box(*id))
                        .collect::<Vec<_>>()
                };
                for leaves in [true, false] {
                    let label = format!(
                        "{case}, the box {}, {nodes} nodes",
                        if leaves { "leaves" } else { "arrives" }
                    );
                    let previous = boxes(&context);
                    let before_local = local_boxes(context.world(), &order);
                    let passes = context.layout_invocations();
                    let mut queue = MutationQueue::new();
                    if leaves {
                        queue.despawn_subtree(leaving);
                    } else {
                        place(&mut queue, document, page, before, arriving);
                    }
                    context.commit_mutations(queue).unwrap();
                    let counters = product_frame(&mut context, document, viewport, &mut shaper);
                    assert_eq!(context.layout_invocations() - passes, 1, "{label}");
                    let moved: Vec<bool> = previous
                        .iter()
                        .zip(boxes(&context))
                        .map(|(old, new)| *old != new)
                        .collect();
                    // At the head every box in the page moved once, at the tail
                    // none did; none of them was on the frontier.
                    assert!(moved[after..].iter().all(|moved| *moved == head), "{label}");
                    assert!(
                        !order[after..]
                            .iter()
                            .any(|id| reflow_oracle::admitted(&context, *id)),
                        "{label}"
                    );
                    let mut cost = Cost::from(counters);
                    cost.translated = carried(context.world(), &order, &before_local, &moved);
                    let shifted = if head { order.len() - after } else { 0 };
                    assert_eq!(cost.origin_only + cost.translated, shifted, "{label}");
                    let moved = moved.iter().filter(|moved| **moved).count();
                    let reached = cost.placed + cost.translated;
                    assert!(
                        reached >= moved && reached <= moved + cost.frontier_placement,
                        "{label}: {moved} boxes moved, {} placed: {cost:?}",
                        cost.placed
                    );
                    // Nothing walked a subtree to measure it.
                    assert_eq!(cost.full_subtrees, 0, "{label}: {cost:?}");
                    let (mut cold, ..) = build(nodes, head, (!leaves).then_some(arriving));
                    assert_matches_cold(&mut context, &mut cold, document);
                    found.push((
                        cost.frontier(),
                        cost.measured,
                        // A group that moved took its cards along: none of them
                        // was walked to place it, nor its plan missed.
                        (cost.children_walked, cost.plan_misses, cost.plan_rebuilds),
                        context.layout_frontier_stats(document),
                    ));
                    costs.push(cost);
                    lookups.push(counters.layout_measure_cache_hits);
                }
            }
            // What was built, walked and measured, and walked to move the groups:
            // the leaving and the arriving, at 1k and at 10k.
            assert_eq!(found[..2], found[2..], "{case}");
            for (small, large) in lookups[..2].iter().zip(&lookups[2..]) {
                assert!(
                    *large <= 2 * small + 8,
                    "{case}: {small} sizes looked up at 1k, {large} at 10k"
                );
            }
            if !head {
                for (small, large) in costs[..2].iter().zip(&costs[2..]) {
                    assert_eq!(
                        small.structural().without_page_result(),
                        large.structural().without_page_result(),
                        "{case}"
                    );
                }
            }
        }
    }
}

pub(super) mod theme_invalidation {
    //! Issue #261: style, theme and component state reach layout only through
    //! the footprint of what they move.
    //!
    //! One document of mixed controls: rows of an auto-sized `Button`, a
    //! `TextInput`, a `Card`, a `ListItem`, and a `Button` in a fixed cell.
    //!
    //! - Gate A: hovering from button to button lays nothing out.
    //! - Gate B: a Light/Dark switch -- a palette, no metric -- shapes no text and
    //!   lays nothing out.
    //! - Gate C: moving the control padding step seeds exactly the buttons that
    //!   use it; text inputs, cards and list rows are not measured, and a button
    //!   in a fixed cell moves nothing above the cell.
    //! - Gate D: one button's padding step moves its own closure, the same at 1k
    //!   and 10k controls, without a fallback.
    //! - Gate E: a thousand writes equal in effect -- spelled differently or
    //!   not -- seed nothing and build no frontier.

    use std::sync::Arc;
    use std::time::Duration;

    use nana_ui_core::{
        ControlPadding, FlexDirection, LayoutStyle, LengthSpec, ThemeAppearance, WorkCounters,
    };

    use crate::layout_engine::verify::skip_layout_verify;
    use crate::world::reflow_oracle::{
        assert_matches_cold, bundled_face_shaper, measured, node, product_frame, styled,
    };
    use crate::world::{DocumentId, NodeKind, StableNodeId, UiWorld};
    use crate::{
        AppContext, Button, Card, LayoutViewport, ListItem, MutationQueue, NanaTextEngineShaper,
        TextInput,
    };

    fn viewport() -> LayoutViewport {
        LayoutViewport::new(1000.0, 800.0)
    }

    /// The controls of one row, as they sit in the world.
    struct Row {
        button: StableNodeId,
        input: StableNodeId,
        card: StableNodeId,
        list_item: StableNodeId,
        cell: StableNodeId,
        row: StableNodeId,
    }

    struct Controls {
        context: AppContext,
        document: DocumentId,
        shaper: NanaTextEngineShaper,
        rows: Vec<Row>,
    }

    impl Controls {
        /// `rows` rows of five controls each.
        fn new(rows: u64) -> Self {
            Self::build(rows, 0.0, None)
        }

        /// The same, laid out first under a control padding step `delta` wider
        /// and, for a `roomy` row, with its free button on the roomy padding
        /// step.
        fn build(rows: u64, delta: f32, roomy: Option<usize>) -> Self {
            let document = DocumentId::new(1).unwrap();
            let mut context = AppContext::new();
            let mut queue = MutationQueue::new();
            queue.create(node(1), document, NodeKind::Document);
            queue.create(node(2), document, NodeKind::Element { tag: "page".into() });
            queue.insert(node(1), node(2), None);
            queue.set_style(
                node(2),
                styled(LayoutStyle {
                    width: Some(LengthSpec::Px(1000.0)),
                    direction: Some(FlexDirection::Column),
                    ..LayoutStyle::default()
                }),
            );
            let mut next = 3;
            let mut take = || {
                next += 1;
                node(next - 1)
            };
            let mut shells = Vec::new();
            for _ in 0..rows {
                let (row, cell) = (take(), take());
                queue.create(row, document, NodeKind::Element { tag: "row".into() });
                queue.insert(node(2), row, None);
                queue.set_style(
                    row,
                    styled(LayoutStyle {
                        direction: Some(FlexDirection::Row),
                        ..LayoutStyle::default()
                    }),
                );
                queue.create(cell, document, NodeKind::Element { tag: "cell".into() });
                queue.set_style(
                    cell,
                    styled(LayoutStyle {
                        width: Some(LengthSpec::Px(140.0)),
                        height: Some(LengthSpec::Px(40.0)),
                        direction: Some(FlexDirection::Row),
                        ..LayoutStyle::default()
                    }),
                );
                shells.push((row, cell));
            }
            context.commit_mutations(queue).unwrap();
            let mut rows = Vec::new();
            let mut queue = MutationQueue::new();
            for (index, (row, cell)) in shells.into_iter().enumerate() {
                let button = context
                    .create_component(document, Button::new(format!("Act {index}")))
                    .unwrap()
                    .stable_id();
                let input = context
                    .create_component(document, TextInput::new(format!("v{index}")))
                    .unwrap()
                    .stable_id();
                let card = context
                    .create_component(document, Card::new())
                    .unwrap()
                    .stable_id();
                let list_item = context
                    .create_component(document, ListItem::new(format!("Row {index}")))
                    .unwrap()
                    .stable_id();
                for id in [button, input, card, list_item, cell] {
                    queue.insert(row, id, None);
                }
                let fixed_button = context
                    .create_component(document, Button::new("Go"))
                    .unwrap()
                    .stable_id();
                queue.insert(cell, fixed_button, None);
                rows.push(Row {
                    button,
                    input,
                    card,
                    list_item,
                    cell,
                    row,
                });
            }
            context.commit_mutations(queue).unwrap();
            let mut controls = Self {
                context,
                document,
                shaper: bundled_face_shaper(),
                rows,
            };
            if delta != 0.0 {
                widen_control_padding(&mut controls.context, delta);
            }
            if let Some(row) = roomy {
                controls.make_roomy(controls.rows[row].button);
            }
            controls.frame();
            controls
        }

        /// Put `button` on the roomy padding step: design intent alone.
        fn make_roomy(&mut self, button: StableNodeId) {
            let mut style = self.world().node_style(button).unwrap().clone();
            style.control_padding_x = Some(ControlPadding::Roomy);
            let mut queue = MutationQueue::new();
            queue.set_style(button, style);
            self.context.commit_mutations(queue).unwrap();
        }

        fn world(&self) -> &UiWorld {
            self.context.world()
        }

        fn frame(&mut self) -> WorkCounters {
            product_frame(
                &mut self.context,
                self.document,
                viewport(),
                &mut self.shaper,
            )
        }
    }

    /// Install the current theme with the control padding step `delta` wider.
    pub(in crate::world::tests) fn widen_control_padding(context: &mut AppContext, delta: f32) {
        let mut model = context.world().style_model();
        model.metrics.control_padding_x += delta;
        let theme = Arc::new(
            context
                .world()
                .installed_theme()
                .clone()
                .with_style_model(model),
        );
        context.set_theme_tokens(theme).unwrap();
    }

    /// What a frame cost layout.
    #[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
    struct Cost {
        seeds: usize,
        frontier_measure: usize,
        frontier_placement: usize,
        measured: usize,
        placed: usize,
        local_fallbacks: usize,
        full_fallbacks: usize,
    }

    impl From<WorkCounters> for Cost {
        fn from(counters: WorkCounters) -> Self {
            Self {
                seeds: counters.layout_frontier_seeds,
                frontier_measure: counters.layout_frontier_nodes_measure,
                frontier_placement: counters.layout_frontier_nodes_placement,
                measured: counters.layout_measure_nodes,
                placed: counters.layout_placement_nodes,
                local_fallbacks: counters.layout_local_subtree_fallbacks,
                full_fallbacks: counters.layout_full_document_fallbacks,
            }
        }
    }

    /// Gate A. Hover moves from button to button across 2,000 controls: paint
    /// work on the two it leaves and enters, and no layout at all.
    #[test]
    fn issue261_hover_paints_and_lays_nothing_out() {
        let _unguarded = skip_layout_verify();
        let mut controls = Controls::new(400);
        let buttons: Vec<StableNodeId> = controls.rows.iter().map(|row| row.button).collect();
        for (step, button) in buttons.iter().take(120).enumerate() {
            controls
                .context
                .set_pointer_hover_at(
                    controls.document,
                    1,
                    Some(*button),
                    Duration::from_millis(16 * step as u64),
                )
                .unwrap();
            let counters = controls.frame();
            assert_eq!(Cost::from(counters), Cost::default(), "hover {step}");
            assert_eq!(counters.component_state_layout_seeds, 0);
            assert_eq!(counters.theme_palette_layout_invalidations, 0);
            // The button it left, the one it entered, and the ancestors their
            // styles inherit from: a row, the page, the document.
            let styles = controls
                .world()
                .last_theme_work_counters()
                .style_nodes_considered;
            assert!(styles <= 2 * 4, "hover {step}: {styles} styles resolved");
        }
    }

    /// Gate B. Light and dark share every metric: a switch is paint. No text
    /// shapes, no box is measured or placed, and the palette queues no layout.
    #[test]
    fn issue261_a_palette_switch_is_paint_only() {
        let _unguarded = skip_layout_verify();
        let mut controls = Controls::new(2_000);
        assert!(controls.world().len() >= 12_000);
        for mode in [
            ThemeAppearance::Light,
            ThemeAppearance::Dark,
            ThemeAppearance::Light,
        ] {
            controls.context.set_preset_theme(mode).unwrap();
            let counters = controls.frame();
            assert_eq!(Cost::from(counters), Cost::default(), "{mode:?}");
            assert_eq!(counters.theme_palette_layout_invalidations, 0);
            assert_eq!(counters.theme_to_layout_seeds, 0);
            let text = controls.world().last_text_work_counters();
            assert_eq!(text.text_nodes_shaped, 0, "{mode:?}");
            assert_eq!(counters.text_constraint_relayouts, 0, "{mode:?}");
        }
    }

    /// Gate C. The control padding step moves: exactly the buttons that use it
    /// are seeded. Text inputs, cards and list rows keep their measurement, and
    /// a button in a fixed cell moves nothing above its cell.
    #[test]
    fn issue261_a_metric_token_reaches_only_its_consumers() {
        let _unguarded = skip_layout_verify();
        let mut controls = Controls::new(2_000);
        let rows = controls.rows.len();
        widen_control_padding(&mut controls.context, 4.0);
        let counters = controls.frame();
        // Two buttons a row use the step: the free one and the one in the cell.
        assert_eq!(counters.theme_metric_dependents_invalidated, 2 * rows);
        assert_eq!(counters.theme_to_layout_seeds, 2 * rows);
        assert_eq!(counters.layout_full_document_fallbacks, 0);
        for row in &controls.rows {
            for other in [row.input, row.card, row.list_item] {
                assert!(!measured(&controls.context, other), "{other:?} measured");
            }
            // The cell keeps its size whatever the button in it does.
            assert!(
                !measured(&controls.context, row.cell),
                "cell {:?} measured",
                row.cell
            );
        }
        let mut cold = Controls::build(2_000, 4.0, None);
        assert_matches_cold(&mut controls.context, &mut cold.context, controls.document);
    }

    /// Gate D. One button's padding step moves: its row and the way up to it,
    /// the same at 1k and 10k controls, with no fallback and no scan of the rest.
    #[test]
    fn issue261_one_state_geometry_change_costs_its_closure() {
        let _unguarded = skip_layout_verify();
        let mut costs = Vec::new();
        for rows in [200u64, 2_000] {
            let mut controls = Controls::new(rows);
            let row = &controls.rows[controls.rows.len() / 2];
            let (button, row_id) = (row.button, row.row);
            let before = controls.world().layout_box(button).unwrap();
            controls.make_roomy(button);
            let counters = controls.frame();
            assert_eq!(counters.style_to_layout_seeds, 1);
            assert_eq!(counters.layout_full_document_fallbacks, 0);
            let after = controls.world().layout_box(button).unwrap();
            assert!(after.width > before.width, "{before:?} -> {after:?}");
            let styles = controls
                .world()
                .last_theme_work_counters()
                .style_nodes_considered;
            assert!(styles <= 4, "{rows} rows: {styles} styles resolved");
            assert!(measured(&controls.context, row_id));
            costs.push(Cost::from(counters));
        }
        assert_eq!(costs[0], costs[1]);
    }

    /// Gate E. A thousand writes equal in effect: the same style again, and the
    /// same layout spelled another way. Nothing is seeded and no frontier built.
    #[test]
    fn issue261_equivalent_writes_seed_nothing() {
        let mut controls = Controls::new(200);
        // A row's own style again, and the page's column axis spelled `None`
        // and `Column` by turns: both lay out as the style already in place.
        let row = controls.rows[10].row;
        let row_style = controls.world().node_style(row).unwrap().clone();
        let page = node(2);
        let page_style = controls.world().node_style(page).unwrap().clone();
        let mut page_respelled = page_style.clone();
        Arc::make_mut(&mut page_respelled.layout).direction = None;
        let before = controls.context.last_work_counters();
        for step in 0..1_000 {
            let mut queue = MutationQueue::new();
            queue.set_style(row, row_style.clone());
            queue.set_style(
                page,
                if step % 2 == 0 {
                    page_respelled.clone()
                } else {
                    page_style.clone()
                },
            );
            controls.context.commit_mutations(queue).unwrap();
        }
        // Nothing to do: no layout seed, no style to resolve, nothing to paint.
        let work = controls.context.take_system_work();
        assert!(work.layout_frontier_seeds.is_empty());
        assert!(work.style.is_empty());
        assert!(work.render_extraction.is_empty());
        // An idle drain leaves the last counters and folds what the writes
        // counted onto them.
        let after = controls.context.last_work_counters();
        assert_eq!(
            after.equivalent_style_layout_skips - before.equivalent_style_layout_skips,
            2_000
        );
        assert_eq!(after.style_to_layout_seeds, before.style_to_layout_seeds);
    }

    /// A recipe patch reprojects the buttons that read it, and changes only the
    /// roles they paint with: nothing is laid out.
    #[test]
    fn issue261_a_button_recipe_patch_lays_nothing_out() {
        let _unguarded = skip_layout_verify();
        let mut controls = Controls::new(400);
        let kind = nana_ui_core::ButtonKind::Ghost;
        let mut definition = nana_ui_core::ThemeDefinition::NANA_DARK;
        let variant = nana_ui_core::ButtonVariantDraft {
            hovered_background: Some(nana_ui_core::SemanticColorRole::Warning),
            ..definition.components.button.variant(kind)
        };
        definition.components.button = definition.components.button.with(kind, variant);
        assert!(
            controls
                .context
                .set_theme_definition(&definition.bump())
                .unwrap()
        );
        let counters = controls.frame();
        assert_eq!(Cost::from(counters), Cost::default());
        assert_eq!(counters.style_to_layout_seeds, 0);
        assert_eq!(counters.theme_to_layout_seeds, 0);
        assert_eq!(
            controls
                .world()
                .node_style(controls.rows[0].button)
                .unwrap()
                .interaction
                .hovered
                .background,
            Some(nana_ui_core::SemanticColorRole::Warning)
        );
    }

    /// A metrics install and a write of intent alone, each pass checked against
    /// a full layout, and the end against the same inputs laid out once.
    #[test]
    fn issue261_metric_and_intent_changes_match_a_full_layout_every_pass() {
        let mut controls = Controls::new(30);
        widen_control_padding(&mut controls.context, 6.0);
        controls.frame();
        controls.make_roomy(controls.rows[7].button);
        controls.frame();
        let mut cold = Controls::build(30, 6.0, Some(7));
        assert_matches_cold(&mut controls.context, &mut cold.context, controls.document);
    }
}

pub(super) mod virtual_scroll {
    //! Issue #262: a virtual list moves by its rows' metric deltas, never by
    //! its logical collection.
    //!
    //! A feed of a million rows sits in a ScrollView forty rows tall, with
    //! eight rows of overscan on each side. Its rows size to their content
    //! ([`VirtualListItems::measured`]): the list mounts its window, reads each
    //! mounted row's height back after a layout pass, and keeps the row index
    //! ([`VirtualListLayout`]) the consumer owns. The consumer owns the data.
    //!
    //! - Gate A: a visible row grows by 8 px. The list looks up no logical row
    //!   (the rows sit at the indices they were placed at while the data holds),
    //!   takes one row's new height, writes a chunk-local, logarithmic slice of
    //!   its index, and moves the rows below; the window stays the window.
    //! - Gate B: a cell's text changes and its row keeps its height: the list's
    //!   extent and every other row's placement stay, and the list does not lay
    //!   out.
    //! - Gate C: the window's 21st row grows: the twenty above keep their
    //!   placement, the rows below shift their origin, and no row below measures
    //!   again -- the same layout work as a row near the window's top.
    //! - Gate D: an unmounted row's data changes: no layout work, whether the
    //!   consumer says the data moved or not; scrolling there shows the new data.
    //! - Gate E: a node in a million-row tree expands with ten children: only
    //!   those ten are created, no mounted row is recreated, and the window is
    //!   all the tree the list looks up.
    //!
    //! The layout guard checks every retained pass against a full layout; the
    //! world holds only the window, so it stays on at a million rows.

    use std::cell::Cell;
    use std::collections::HashMap;
    use std::rc::Rc;
    use std::sync::Arc;

    use nana_ui_core::{LayoutStyle, LengthSpec, VirtualTreeLayout, VirtualTreeRow, WorkCounters};

    use crate::world::reflow_oracle::{bundled_face_shaper, product_frame};
    use crate::world::{DocumentId, StableNodeId};
    use crate::{
        AppContext, Entity, LayoutBox, LayoutViewport, List, NanaTextEngineShaper, NodeStyle,
        ScrollAxes, ScrollOffset, ScrollView, Text, VirtualListItems, VirtualListLayout,
        VirtualTreeItems,
    };

    pub(in crate::world::tests) const ROW: f32 = 24.0;
    const VISIBLE: usize = 40;
    const OVERSCAN: usize = 8;
    /// The most rows a window can hold: the visible rows, the overscan on each
    /// side, and a row cut by each edge.
    const WINDOW: usize = VISIBLE + 2 * OVERSCAN + 2;

    fn document() -> DocumentId {
        DocumentId::new(1).unwrap()
    }

    pub(in crate::world::tests) fn viewport() -> LayoutViewport {
        LayoutViewport::new(320.0, VISIBLE as f32 * ROW)
    }

    /// A ScrollView the size of the viewport holding an empty List.
    pub(in crate::world::tests) fn port(cx: &mut AppContext) -> (Entity<ScrollView>, Entity<List>) {
        let scroll = cx
            .create_component(document(), ScrollView::new(ScrollAxes::Vertical))
            .unwrap();
        cx.update_component(scroll, |scroll, _| {
            let layout = Arc::make_mut(&mut scroll.style.layout);
            layout.width = Some(LengthSpec::Px(320.0));
            layout.height = Some(LengthSpec::Px(VISIBLE as f32 * ROW));
        })
        .unwrap();
        // The list spans the port, as a feed does: its rows are as wide.
        let list = cx
            .create_component(
                document(),
                List::new().style(NodeStyle {
                    layout: Arc::new(LayoutStyle {
                        width: Some(LengthSpec::Percent(100.0)),
                        ..LayoutStyle::default()
                    }),
                    ..NodeStyle::default()
                }),
            )
            .unwrap();
        cx.append_child(scroll, list).unwrap();
        (scroll, list)
    }

    fn sized(height: f32) -> NodeStyle {
        NodeStyle {
            layout: Arc::new(LayoutStyle {
                height: Some(LengthSpec::Px(height)),
                ..LayoutStyle::default()
            }),
            ..NodeStyle::default()
        }
    }

    /// The consumer's data for a row: what its cell says and how tall its
    /// content lays out.
    #[derive(Debug, Clone, PartialEq)]
    pub(in crate::world::tests) struct Row {
        pub(in crate::world::tests) value: &'static str,
        pub(in crate::world::tests) height: f32,
    }

    impl Default for Row {
        fn default() -> Self {
            Self {
                value: "99",
                height: ROW,
            }
        }
    }

    fn row_view(row: Row) -> Text {
        Text::new(row.value).style(sized(row.height))
    }

    /// What one virtual list pass cost, from [`WorkCounters`].
    #[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
    pub(in crate::world::tests) struct Pass {
        pub(in crate::world::tests) metric_updates: usize,
        pub(in crate::world::tests) index_updates: usize,
        pub(in crate::world::tests) remeasured: usize,
        pub(in crate::world::tests) repositioned: usize,
        pub(in crate::world::tests) scanned: usize,
        pub(in crate::world::tests) extent_updates: usize,
        pub(in crate::world::tests) materialized: usize,
    }

    impl Pass {
        fn between(before: &WorkCounters, after: &WorkCounters) -> Self {
            Self {
                metric_updates: after.virtual_row_metric_updates
                    - before.virtual_row_metric_updates,
                index_updates: after.virtual_prefix_index_updates
                    - before.virtual_prefix_index_updates,
                remeasured: after.virtual_rows_remeasured - before.virtual_rows_remeasured,
                repositioned: after.virtual_rows_repositioned - before.virtual_rows_repositioned,
                scanned: after.virtual_logical_rows_scanned - before.virtual_logical_rows_scanned,
                extent_updates: after.virtual_scroll_extent_updates
                    - before.virtual_scroll_extent_updates,
                materialized: after.virtual_rows_materialized_from_layout
                    - before.virtual_rows_materialized_from_layout,
            }
        }
    }

    /// A measured feed of `rows` rows.
    pub(in crate::world::tests) struct Feed {
        pub(in crate::world::tests) cx: AppContext,
        shaper: NanaTextEngineShaper,
        pub(in crate::world::tests) scroll: Entity<ScrollView>,
        list: Entity<List>,
        pub(in crate::world::tests) items: VirtualListItems<u64, Text>,
        layout: VirtualListLayout,
        rows: usize,
        /// Rows whose data is not the default.
        data: HashMap<u64, Row>,
        /// The list's fingerprint: bumped when keys move between indices.
        fingerprint: u64,
        /// Logical rows the list asked the consumer for.
        pub(in crate::world::tests) lookups: Rc<Cell<usize>>,
    }

    impl Feed {
        pub(in crate::world::tests) fn new(rows: usize) -> Self {
            let mut cx = AppContext::new();
            let (scroll, list) = port(&mut cx);
            let mut feed = Self {
                cx,
                shaper: bundled_face_shaper(),
                scroll,
                list,
                items: VirtualListItems::measured(),
                // A million rows estimated alike: a count per chunk.
                layout: VirtualListLayout::uniform(rows, ROW),
                rows,
                data: HashMap::new(),
                fingerprint: 0,
                lookups: Rc::new(Cell::new(0)),
            };
            // The port lays out first: the list's window is its viewport.
            feed.frame();
            feed.settle();
            feed
        }

        pub(in crate::world::tests) fn frame(&mut self) -> WorkCounters {
            product_frame(&mut self.cx, document(), viewport(), &mut self.shaper)
        }

        /// One sync of the list, as a frame hook runs it: what it cost.
        pub(in crate::world::tests) fn sync(&mut self) -> Pass {
            let before = self.cx.last_work_counters();
            let Self {
                cx,
                scroll,
                list,
                items,
                layout,
                rows,
                data,
                fingerprint,
                lookups,
                ..
            } = self;
            let rows = *rows;
            let (at, of) = (Rc::clone(lookups), Rc::clone(lookups));
            cx.sync_virtual_list_measured_with(
                *scroll,
                *list,
                items,
                layout,
                OVERSCAN as f32 * ROW,
                *fingerprint,
                &[],
                move |index| {
                    at.set(at.get() + 1);
                    index as u64
                },
                move |key| {
                    of.set(of.get() + 1);
                    usize::try_from(*key).ok().filter(|index| *index < rows)
                },
                |_, key| row_view(data.get(key).cloned().unwrap_or_default()),
                |_, _, _, _| Ok(()),
            )
            .unwrap();
            Pass::between(&before, &self.cx.last_work_counters())
        }

        /// Sync and lay out until a sync commits nothing.
        pub(in crate::world::tests) fn settle(&mut self) {
            for _ in 0..8 {
                let generation = self.cx.world().generation();
                self.sync();
                if self.cx.world().generation() == generation {
                    return;
                }
                self.frame();
            }
            panic!("the feed keeps moving");
        }

        pub(in crate::world::tests) fn scroll_to_row(&mut self, index: usize) {
            self.cx
                .scroll_to(
                    self.scroll,
                    ScrollOffset {
                        x: 0.0,
                        y: index as f32 * ROW,
                    },
                )
                .unwrap();
            self.settle();
        }

        pub(in crate::world::tests) fn mounted(&self) -> Vec<u64> {
            self.items.mounted_keys().to_vec()
        }

        fn entity(&self, key: u64) -> Entity<Text> {
            self.items.entity(&key).expect("a mounted row")
        }

        /// The consumer's data for `key` changes. A mounted row shows it at
        /// once; its key stays at its index, so the fingerprint stays.
        pub(in crate::world::tests) fn edit(&mut self, key: u64, row: Row) {
            self.data.insert(key, row.clone());
            if let Some(entity) = self.items.entity(&key) {
                self.cx
                    .update_component(entity, |text, _| *text = row_view(row))
                    .unwrap();
            }
        }

        /// The box of each mounted row's content, in window order.
        pub(in crate::world::tests) fn boxes(&self) -> Vec<LayoutBox> {
            self.mounted()
                .into_iter()
                .map(|key| {
                    self.cx
                        .world()
                        .layout_box(self.entity(key).stable_id())
                        .unwrap()
                })
                .collect()
        }

        fn node_count(&self) -> usize {
            self.cx.world().document_order(document()).len()
        }
    }

    /// A visible row grows by 8 px: a sync that reads it back and moves the rows
    /// below, and the frames around it.
    pub(in crate::world::tests) struct Growth {
        pub(in crate::world::tests) laid: WorkCounters,
        pub(in crate::world::tests) pass: Pass,
        pub(in crate::world::tests) placed: WorkCounters,
        pub(in crate::world::tests) lookups: usize,
    }

    /// Grow the row at `at` in the window by 8 px.
    pub(in crate::world::tests) fn grow(feed: &mut Feed, at: usize) -> Growth {
        let key = feed.mounted()[at];
        feed.edit(
            key,
            Row {
                height: ROW + 8.0,
                ..Row::default()
            },
        );
        let lookups = feed.lookups.get();
        let laid = feed.frame();
        let pass = feed.sync();
        let placed = feed.frame();
        let lookups = feed.lookups.get() - lookups;
        feed.settle();
        Growth {
            laid,
            pass,
            placed,
            lookups,
        }
    }

    #[test]
    fn issue262_gate_a_a_row_growing_in_a_million_scans_its_window_at_most() {
        let mut feed = Feed::new(1_000_000);
        feed.scroll_to_row(500_000);
        let window = feed.mounted();
        assert!(
            window.len() <= WINDOW,
            "{} rows mounted for a window of {WINDOW}",
            window.len()
        );
        // The retained tree is the window: a placement container and a cell per
        // row, the port, the list, and the port's own chrome.
        let nodes = feed.node_count();
        assert!(nodes <= 2 * WINDOW + 8, "{nodes} nodes for {WINDOW} rows");

        let at = 10;
        let before = feed.boxes();
        let growth = grow(&mut feed, at);
        let after = feed.boxes();

        assert_eq!(feed.mounted(), window, "the window is the window");
        assert!(
            growth.pass.scanned <= 64 && growth.lookups <= 64,
            "{:?}, {} lookups",
            growth.pass,
            growth.lookups
        );
        assert_eq!(growth.pass.remeasured, 1, "{:?}", growth.pass);
        assert_eq!(growth.pass.metric_updates, 1, "{:?}", growth.pass);
        // The grown row's chunk stops being one extent (its 512 rows, once),
        // then O(log C) index entries over C chunks: never a million.
        assert!(growth.pass.index_updates <= 512 + 32, "{:?}", growth.pass);
        assert_eq!(growth.pass.repositioned, window.len() - at - 1);
        assert_eq!(growth.pass.extent_updates, 1);
        assert_eq!(growth.pass.materialized, 0);
        // The row lays out at its new height: its cell and its row measure.
        assert!(
            growth.laid.layout_measure_nodes <= 2,
            "{}",
            growth.laid.layout_measure_nodes
        );
        // The rows below shift their origin; none of them measures. What the
        // pass measures is the list, whose height moved.
        assert!(
            growth.placed.layout_measure_nodes <= 2,
            "{}",
            growth.placed.layout_measure_nodes
        );
        assert_eq!(growth.placed.intrinsic_measure_full_subtrees, 0);
        // Each row below moves, and what it holds moves with it unplaced and
        // unmarked: each row is the root of a translation.
        assert!(growth.placed.layout_translations >= growth.pass.repositioned);
        assert_eq!(growth.placed.layout_translated_nodes, 0);
        assert_eq!(growth.placed.layout_full_document_fallbacks, 0);
        for (index, (before, after)) in before.iter().zip(&after).enumerate() {
            let (moved, grew) = match index.cmp(&at) {
                std::cmp::Ordering::Less => (0.0, 0.0),
                std::cmp::Ordering::Equal => (0.0, 8.0),
                std::cmp::Ordering::Greater => (8.0, 0.0),
            };
            assert_eq!(after.y - before.y, moved, "row {index} moved");
            assert_eq!(after.height - before.height, grew, "row {index} grew");
        }

        // A second row in the same chunk writes only index entries.
        let again = grow(&mut feed, at + 1);
        assert!(again.pass.index_updates <= 32, "{:?}", again.pass);
    }

    #[test]
    fn issue262_gate_b_a_cell_that_keeps_its_height_moves_nothing() {
        let mut feed = Feed::new(1_000_000);
        feed.scroll_to_row(500_000);
        let key = feed.mounted()[10];
        let before = feed.boxes();
        feed.edit(
            key,
            Row {
                value: "98",
                ..Row::default()
            },
        );
        let laid = feed.frame();
        // The cell shapes and measures again; its row and the list do not.
        assert!(
            laid.layout_measure_nodes <= 1,
            "{}",
            laid.layout_measure_nodes
        );
        let generation = feed.cx.world().generation();
        let pass = feed.sync();
        assert_eq!(pass, Pass::default(), "the list has nothing to move");
        assert_eq!(feed.cx.world().generation(), generation, "nothing commits");
        let passes = feed.cx.layout_invocations();
        feed.frame();
        assert_eq!(feed.cx.layout_invocations(), passes, "nothing lays out");
        assert_eq!(feed.boxes(), before);
    }

    #[test]
    fn issue262_gate_c_rows_above_a_grown_row_stay_and_rows_below_shift() {
        let mut feed = Feed::new(1_000_000);
        feed.scroll_to_row(500_000);
        let window = feed.mounted().len();
        let containers = |feed: &Feed| {
            feed.mounted()
                .into_iter()
                .map(|key| {
                    let id = feed.entity(key).stable_id();
                    let container = feed.cx.world().parent_id(id).unwrap();
                    feed.cx
                        .world()
                        .node_style(container)
                        .unwrap()
                        .layout
                        .clone()
                })
                .collect::<Vec<_>>()
        };
        let placements = containers(&feed);
        let before = feed.boxes();
        let late = grow(&mut feed, 20);
        let after = feed.boxes();
        let placed = containers(&feed);

        // Rows 0..19 of the window: their placement is the one they had.
        for index in 0..20 {
            assert!(
                Arc::ptr_eq(&placements[index], &placed[index]),
                "row {index} was placed again"
            );
            assert_eq!(after[index], before[index]);
        }
        // Rows after 20 shift their origin and keep their size.
        assert_eq!(late.pass.repositioned, window - 21);
        for index in 21..window {
            assert_eq!(after[index].y - before[index].y, 8.0, "row {index}");
            assert_eq!(after[index].height, before[index].height, "row {index}");
        }

        // A row near the window's top moves more rows the same way, and lays out
        // exactly as much: no row below a grown row measures.
        let mut feed = Feed::new(1_000_000);
        feed.scroll_to_row(500_000);
        let early = grow(&mut feed, 2);
        assert_eq!(early.pass.repositioned, window - 3);
        assert_eq!(
            early.placed.layout_measure_nodes,
            late.placed.layout_measure_nodes
        );
        assert_eq!(
            early.laid.layout_measure_nodes,
            late.laid.layout_measure_nodes
        );
    }

    #[test]
    fn issue262_gate_d_an_unmounted_row_changes_without_layout() {
        let mut feed = Feed::new(1_000_000);
        feed.scroll_to_row(500_000);
        let far = 900_000u64;
        let row = Row {
            value: "98",
            height: 48.0,
        };
        let passes = feed.cx.layout_invocations();

        // The consumer knows the change moved no key: the range gate holds.
        feed.edit(far, row.clone());
        let generation = feed.cx.world().generation();
        let pass = feed.sync();
        assert_eq!(pass, Pass::default());
        assert_eq!(feed.cx.world().generation(), generation);
        feed.frame();
        assert_eq!(feed.cx.layout_invocations(), passes, "no layout work");

        // A consumer that bumps its fingerprint on any change: the mounted rows
        // are looked up again to be measured, and the window is keyed again --
        // twice the window, never the collection -- and nothing commits.
        feed.fingerprint += 1;
        let pass = feed.sync();
        assert!(pass.scanned <= 2 * WINDOW, "{pass:?}");
        assert_eq!(Pass { scanned: 0, ..pass }, Pass::default());
        assert_eq!(feed.cx.world().generation(), generation);
        feed.frame();
        assert_eq!(feed.cx.layout_invocations(), passes, "no layout work");

        // Scrolled into the window, the row shows the data as it is now.
        feed.scroll_to_row(far as usize);
        let entity = feed.entity(far);
        assert_eq!(
            feed.cx.read(entity, |text| text.value.clone()).unwrap(),
            "98"
        );
        let at = feed.mounted().iter().position(|key| *key == far).unwrap();
        assert_eq!(feed.boxes()[at].height, 48.0);
    }

    #[test]
    fn issue262_gate_e_expanding_a_node_in_a_million_creates_only_its_children() {
        const NODES: usize = 1_000_000;
        const CHILDREN: usize = 10;
        let mut cx = AppContext::new();
        let mut shaper = bundled_face_shaper();
        let (scroll, list) = port(&mut cx);
        let mut tree = VirtualTreeLayout::uniform(ROW, std::iter::repeat_n(0, NODES));
        let mut items = VirtualTreeItems::<u64, Text>::default();
        // The node at `parent` holds its children after it once expanded. Keys
        // are node ids; children are numbered from 2,000,000.
        let expanded: Cell<Option<usize>> = Cell::new(None);
        let lookups = Cell::new(0usize);
        let key_at = |index: usize| -> u64 {
            lookups.set(lookups.get() + 1);
            match expanded.get() {
                Some(parent) if index > parent && index <= parent + CHILDREN => {
                    2_000_000 + (index - parent - 1) as u64
                }
                Some(parent) if index > parent + CHILDREN => (index - CHILDREN) as u64,
                _ => index as u64,
            }
        };
        let index_of_key = |key: &u64| -> Option<usize> {
            lookups.set(lookups.get() + 1);
            let key = usize::try_from(*key).ok()?;
            match expanded.get() {
                Some(parent) if key >= 2_000_000 => Some(parent + 1 + key - 2_000_000),
                Some(parent) if key > parent => Some(key + CHILDREN),
                _ => (key < NODES).then_some(key),
            }
        };
        let mut fingerprint = 0;
        let sync = |cx: &mut AppContext,
                    items: &mut VirtualTreeItems<u64, Text>,
                    tree: &VirtualTreeLayout,
                    fingerprint: u64| {
            let before = cx.last_work_counters();
            cx.sync_virtual_tree_retained_in(
                scroll,
                list,
                items,
                tree,
                OVERSCAN as f32 * ROW,
                fingerprint,
                &[],
                key_at,
                index_of_key,
                |_, key| Text::new(format!("node {key}")),
            )
            .unwrap();
            Pass::between(&before, &cx.last_work_counters())
        };
        let frame = |cx: &mut AppContext, shaper: &mut NanaTextEngineShaper| -> WorkCounters {
            product_frame(cx, document(), viewport(), shaper)
        };
        frame(&mut cx, &mut shaper);
        sync(&mut cx, &mut items, &tree, fingerprint);
        frame(&mut cx, &mut shaper);
        cx.scroll_to(
            scroll,
            ScrollOffset {
                x: 0.0,
                y: 500_000.0 * ROW,
            },
        )
        .unwrap();
        sync(&mut cx, &mut items, &tree, fingerprint);
        frame(&mut cx, &mut shaper);

        let mounted = items.mounted_keys().to_vec();
        let entities: HashMap<u64, StableNodeId> = mounted
            .iter()
            .map(|key| (*key, items.entity(key).unwrap().stable_id()))
            .collect();
        let parent = 500_010;
        assert!(mounted.contains(&(parent as u64)));
        let lookups_before = lookups.get();

        assert!(tree.expand(
            parent,
            std::iter::repeat_n(
                VirtualTreeRow {
                    extent: ROW,
                    descendant_count: 0,
                },
                CHILDREN,
            ),
        ));
        expanded.set(Some(parent));
        fingerprint += 1;
        let pass = sync(&mut cx, &mut items, &tree, fingerprint);
        let laid = frame(&mut cx, &mut shaper);

        let now = items.mounted_keys().to_vec();
        assert_eq!(now.len(), mounted.len(), "the window holds as many rows");
        let created: Vec<u64> = now
            .iter()
            .copied()
            .filter(|key| !entities.contains_key(key))
            .collect();
        assert_eq!(
            created,
            (0..CHILDREN as u64)
                .map(|child| 2_000_000 + child)
                .collect::<Vec<_>>(),
            "only the children are created"
        );
        assert_eq!(pass.materialized, CHILDREN, "{pass:?}");
        // Every row that stayed is the row it was: no lifecycle churn.
        for key in now.iter().filter(|key| entities.contains_key(key)) {
            assert_eq!(
                items.entity(key).unwrap().stable_id(),
                entities[key],
                "row {key} was recreated"
            );
        }
        // The window is all of the tree the list looked up.
        let looked_up = lookups.get() - lookups_before;
        assert!(
            pass.scanned <= WINDOW && looked_up <= WINDOW,
            "{pass:?}, {looked_up} lookups"
        );
        // Ten rows join one chunk of a count: O(log C) index entries.
        assert!(pass.index_updates <= 32, "{pass:?}");
        assert_eq!(pass.extent_updates, 1);
        // The children lay out in full, once each, and nothing else does: the
        // rows below the parent move by their origin, at a height of their own,
        // and what each holds moves with it unplaced.
        assert_eq!(laid.intrinsic_measure_full_subtrees, CHILDREN);
        assert!(
            laid.layout_translations >= pass.repositioned,
            "{} rows moved for {pass:?}",
            laid.layout_translations
        );
        assert_eq!(laid.layout_translated_nodes, 0);
        assert_eq!(laid.layout_full_document_fallbacks, 0);
    }

    /// A table's cell changes what it says and keeps its size: the edit stays in
    /// the cell. Its row, the other cells and the table lay nothing out, and the
    /// table's window is not placed again.
    #[test]
    fn issue262_a_table_cell_edit_stays_in_its_cell() {
        use crate::{Table, TableCell, TableRow, VirtualTableItems};
        use nana_ui_core::{TableColumn, VirtualTableLayout, VirtualViewport};

        let mut cx = AppContext::new();
        let mut shaper = bundled_face_shaper();
        let table = cx.create_component(document(), Table::new()).unwrap();
        let layout = VirtualTableLayout::new(
            std::iter::repeat_n(ROW, 1_000_000),
            (0..1_000).map(|column| TableColumn::new(column.to_string(), 80.0)),
        );
        let mut items = VirtualTableItems::<usize, usize>::default();
        let window = VirtualViewport {
            offset: [0.0, 500_000.0 * ROW],
            extent: [320.0, VISIBLE as f32 * ROW],
            overscan: [0.0; 2],
        };
        let fill = |cx: &mut AppContext, items: &mut VirtualTableItems<usize, usize>| {
            cx.materialize_virtual_table_retained_in(
                table,
                items,
                &layout,
                window,
                [0, 0],
                &[],
                |index| index,
                |key| Some(*key),
                |index| index,
                |key| Some(*key),
                |_, _| TableRow::new(),
                |_, _, _, _| TableCell::new("99"),
            )
            .unwrap();
        };
        fill(&mut cx, &mut items);
        product_frame(&mut cx, document(), viewport(), &mut shaper);
        let order = cx.world().document_order(document());
        let boxes = |cx: &AppContext| {
            order
                .iter()
                .map(|id| cx.world().layout_box(*id))
                .collect::<Vec<_>>()
        };
        let before = boxes(&cx);
        let cell = items.cell_entity(&500_010, &2).unwrap();
        cx.update_component(cell, |cell, _| cell.value = "98".into())
            .unwrap();
        let laid = product_frame(&mut cx, document(), viewport(), &mut shaper);
        // The cell shaped its new text in the box it has; nothing lays out.
        assert!(laid.text_shaped >= 1, "the cell shaped again");
        assert_eq!(laid.layout_frontier_seeds, 0);
        assert_eq!(laid.layout_measure_nodes, 0);
        assert_eq!(laid.layout_placement_nodes, 0);
        assert_eq!(boxes(&cx), before);
        let generation = cx.world().generation();
        fill(&mut cx, &mut items);
        assert_eq!(
            cx.world().generation(),
            generation,
            "the window stays placed"
        );
    }
}

mod resize_propagation {
    //! Issue #264: a resize reaches the boxes that consume the constraint that
    //! changed, and nothing else.
    //!
    //! One workspace: a fixed dock on the left -- the filler that takes the
    //! document from 1k to 100k nodes -- and a real [`SplitPane`] beside it. Its
    //! first pane holds a fill-width toolbar of fixed buttons, a list of
    //! fill-width rows, a wrapping paragraph, a fixed box with a subtree of its
    //! own and a box whose height is half the pane's; the second holds a
    //! toolbar, a paragraph and fixed cards. The splitter drags through
    //! `SplitPaneMutation::SetSize`, which writes the first pane's width.
    //!
    //! - Gate A: 240 drags in a 10k workspace. No fixed box, and no box of the
    //!   dock, is measured; no text shapes again; the nodes are the same at the
    //!   end; every frame matches a cold layout.
    //! - Gate B: a width-only change never measures the box that reads only the
    //!   pane's height, and the paragraph rewraps from its shaped runs; a
    //!   height-only change never measures the fill-width rows.
    //! - Gate C: the fixed box and everything under it never measure.
    //! - Gate D: a drag costs the same in a 1k and a 100k workspace, and asks
    //!   the same children whether they consume the constraint.
    //! - A viewport resize reaches the roots and the boxes that consume the
    //!   axis it moved on, and costs the same at 1k and 100k nodes. A `vw` box
    //!   is not reached by a height-only resize, nor a `vh` one by a width-only
    //!   one.
    //! - The pane beside the dragged one is a constraint seed too: it narrows
    //!   as the first widens.
    //! - A real `Dock` dragged through its split ratio measures the boxes that
    //!   read a frame's width and not the ones that read only its height.
    //!
    //! "Measured" is what the layout computed, past its memos
    //! (`layout_engine::measure_trace`), not what the frontier admitted: a box
    //! can be measured again because its memo key moved.
    //!
    //! - Gate E: an elastic toolbar in the first pane, dragged inside one cost
    //!   level of its solve, builds no envelope and solves nothing from scratch:
    //!   each frame re-shares the held solve, its fixed boxes never measure, and
    //!   every frame's boxes match a cold layout's.

    use nana_ui_core::{
        AlignSpec, FlexDirection, InvalidationKind, LayoutStyle, LengthSpec, SplitAxis,
        SplitPaneModel, SplitPaneMutation, WorkCounters,
    };

    use std::collections::HashSet;

    use crate::layout_engine::measure_trace;
    use crate::layout_engine::verify::skip_layout_verify;
    use crate::world::reflow_oracle::{
        Builder, admitted, assert_matches_cold, bundled_face_shaper, fixed, product_frame,
        resize_frame, styled,
    };
    use crate::world::{DocumentId, NodeKind, StableNodeId, UiWorld};
    use crate::{
        AppContext, Entity, LayoutViewport, MutationQueue, NanaTextEngineShaper, SplitPane,
    };

    const PARAGRAPH: &str =
        "a paragraph long enough to wrap over a few lines of the pane it sits in";
    const FIRST_SIZE: f32 = 300.0;

    fn viewport() -> LayoutViewport {
        LayoutViewport::new(1200.0, 800.0)
    }

    fn fill_column() -> LayoutStyle {
        LayoutStyle {
            width: Some(LengthSpec::Fill),
            direction: Some(FlexDirection::Column),
            ..LayoutStyle::default()
        }
    }

    /// A pane of the split: it fills the split on both axes.
    fn pane() -> LayoutStyle {
        LayoutStyle {
            width: Some(LengthSpec::Fill),
            height: Some(LengthSpec::Fill),
            direction: Some(FlexDirection::Column),
            ..LayoutStyle::default()
        }
    }

    /// A fill-width toolbar of ten fixed buttons.
    fn toolbar(b: &mut Builder, parent: StableNodeId) -> Vec<StableNodeId> {
        let bar = b.element(
            parent,
            LayoutStyle {
                width: Some(LengthSpec::Fill),
                height: Some(LengthSpec::Px(32.0)),
                direction: Some(FlexDirection::Row),
                ..LayoutStyle::default()
            },
        );
        (0..10)
            .map(|_| {
                let button = b.element(bar, fixed(28.0, 28.0));
                b.label(button, "B");
                button
            })
            .collect()
    }

    /// Nodes the gates name.
    struct Parts {
        dock: StableNodeId,
        /// Fixed boxes of both panes: toolbar buttons and cards.
        fixed: Vec<StableNodeId>,
        /// A fixed box with a subtree of its own.
        fixed_box: StableNodeId,
        /// A box whose width is fixed and whose height is half the pane's.
        tall: StableNodeId,
        /// Fill-width rows of the first pane's list.
        rows: Vec<StableNodeId>,
        paragraph: StableNodeId,
        /// The elastic toolbar's items and their fixed boxes, when there is one.
        elastic_items: Vec<StableNodeId>,
        elastic_boxes: Vec<StableNodeId>,
    }

    struct Workspace {
        context: AppContext,
        document: DocumentId,
        shaper: NanaTextEngineShaper,
        split: Entity<SplitPane>,
        parts: Parts,
        size: f32,
        /// The nodes the last frame measured.
        measured: HashSet<StableNodeId>,
    }

    impl Workspace {
        /// A 1200x800 page: a dock of `dock_cards` fixed cards, and the split.
        fn new(dock_cards: u64, size: f32) -> Self {
            Self::build(dock_cards, size, false, false)
        }

        /// The same, on a page that fills the viewport.
        fn fluid(dock_cards: u64) -> Self {
            Self::build(dock_cards, FIRST_SIZE, true, false)
        }

        /// The same page with an elastic toolbar at the top of the first pane: a
        /// row that solves its overflow, six padded items around fixed boxes.
        fn elastic(dock_cards: u64, size: f32) -> Self {
            Self::build(dock_cards, size, false, true)
        }

        fn build(dock_cards: u64, size: f32, fluid: bool, elastic: bool) -> Self {
            let document = DocumentId::new(1).unwrap();
            let (mut b, root) = Builder::new(document, 1);
            let (width, height) = if fluid {
                (LengthSpec::Fill, LengthSpec::Fill)
            } else {
                (LengthSpec::Px(1200.0), LengthSpec::Px(800.0))
            };
            let page = b.element(
                root,
                LayoutStyle {
                    width: Some(width),
                    height: Some(height),
                    direction: Some(FlexDirection::Row),
                    align_items: AlignSpec::Stretch,
                    ..LayoutStyle::default()
                },
            );
            let dock = b.element(
                page,
                LayoutStyle {
                    width: Some(LengthSpec::Px(300.0)),
                    direction: Some(FlexDirection::Column),
                    ..LayoutStyle::default()
                },
            );
            for _ in 0..dock_cards {
                let card = b.element(dock, fixed(280.0, 24.0));
                b.label(card, "dock");
            }
            let mut fixed_boxes = Vec::new();
            let first = b.detached(pane());
            let mut elastic_items = Vec::new();
            let mut elastic_boxes = Vec::new();
            if elastic {
                let bar = b.element(
                    first,
                    LayoutStyle {
                        width: Some(LengthSpec::Fill),
                        height: Some(LengthSpec::Px(40.0)),
                        direction: Some(FlexDirection::Row),
                        gap: Some(LengthSpec::Px(8.0)),
                        adaptation: Some(
                            crate::world::tests::dynamic_layout::toolbar_fixture::gap_profile()
                                .into(),
                        ),
                        ..LayoutStyle::default()
                    },
                );
                for _ in 0..6 {
                    let item = b.element(
                        bar,
                        LayoutStyle {
                            direction: Some(FlexDirection::Row),
                            padding_left: Some(LengthSpec::Px(16.0)),
                            padding_right: Some(LengthSpec::Px(16.0)),
                            adaptation: Some(crate::world::tests::dynamic_layout::toolbar_fixture::padding_profile().into()),
                            ..LayoutStyle::default()
                        },
                    );
                    elastic_boxes.push(b.element(item, fixed(40.0, 20.0)));
                    elastic_items.push(item);
                }
            }
            fixed_boxes.extend(toolbar(&mut b, first));
            let list = b.element(first, fill_column());
            let rows = (0..40)
                .map(|index| {
                    let row = b.element(list, fill_column());
                    b.label(row, &format!("Item {index}"));
                    row
                })
                .collect();
            let text_box = b.element(first, fill_column());
            let paragraph = b.label(text_box, PARAGRAPH);
            let fixed_box = b.element(
                first,
                LayoutStyle {
                    direction: Some(FlexDirection::Column),
                    ..fixed(200.0, 100.0)
                },
            );
            for _ in 0..4 {
                let row = b.element(fixed_box, fill_column());
                b.label(row, "inside");
            }
            let tall = b.element(
                first,
                LayoutStyle {
                    width: Some(LengthSpec::Px(80.0)),
                    height: Some(LengthSpec::Percent(50.0)),
                    ..LayoutStyle::default()
                },
            );
            let second = b.detached(pane());
            fixed_boxes.extend(toolbar(&mut b, second));
            let text_box = b.element(second, fill_column());
            b.label(text_box, PARAGRAPH);
            for _ in 0..20 {
                let card = b.element(second, fixed(200.0, 40.0));
                b.label(card, "card");
                fixed_boxes.push(card);
            }
            let mut context = AppContext::new();
            context.commit_mutations(b.queue).unwrap();
            let handle = context
                .create_view(
                    document,
                    NodeKind::Element {
                        tag: "handle".into(),
                    },
                    (),
                )
                .unwrap()
                .stable_id();
            let mut model = SplitPaneModel::new(SplitAxis::Horizontal, FIRST_SIZE, 100.0, 800.0);
            model.update(SplitPaneMutation::SetSize(size));
            let split = context
                .create_component(
                    document,
                    SplitPane::from_model(&model, first, second).handle(handle),
                )
                .unwrap();
            context.assemble_split_pane(split).unwrap();
            let mut queue = MutationQueue::new();
            queue.insert(page, split.stable_id(), None);
            context.commit_mutations(queue).unwrap();
            let mut workspace = Self {
                context,
                document,
                shaper: bundled_face_shaper(),
                split,
                parts: Parts {
                    dock,
                    fixed: fixed_boxes,
                    fixed_box,
                    tall,
                    rows,
                    paragraph,
                    elastic_items,
                    elastic_boxes,
                },
                size,
                measured: HashSet::new(),
            };
            workspace.frame();
            workspace
        }

        fn world(&self) -> &UiWorld {
            self.context.world()
        }

        fn frame(&mut self) -> WorkCounters {
            measure_trace::begin();
            let counters = product_frame(
                &mut self.context,
                self.document,
                viewport(),
                &mut self.shaper,
            );
            self.measured = measure_trace::take();
            counters
        }

        fn resize(&mut self, viewport: LayoutViewport) -> WorkCounters {
            measure_trace::begin();
            let counters =
                resize_frame(&mut self.context, self.document, viewport, &mut self.shaper);
            self.measured = measure_trace::take();
            counters
        }

        /// Whether the last frame measured `id`.
        fn measured(&self, id: StableNodeId) -> bool {
            self.measured.contains(&id)
        }

        /// Drag the splitter to `size`, one frame.
        fn drag_to(&mut self, size: f32) -> WorkCounters {
            self.size = size;
            self.context
                .update_component(self.split, |pane, _| {
                    pane.apply(SplitPaneMutation::SetSize(size))
                })
                .unwrap();
            self.frame()
        }

        fn subtree(&self, root: StableNodeId) -> Vec<StableNodeId> {
            let mut ids = Vec::new();
            let mut stack = vec![root];
            while let Some(id) = stack.pop() {
                ids.push(id);
                stack.extend(self.world().node(id).unwrap().children);
            }
            ids
        }
    }

    /// What a frame cost layout, in the units the gates name.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    struct Cost {
        seeds: usize,
        frontier_measure: usize,
        frontier_placement: usize,
        contexts: usize,
        edges: usize,
        measured: usize,
        placed: usize,
        children_walked: usize,
        full_subtrees: usize,
        constraint_seeds: usize,
        considered: usize,
        remeasured: usize,
        skipped: usize,
        context_solves: usize,
        local_fallbacks: usize,
        full_fallbacks: usize,
    }

    impl From<WorkCounters> for Cost {
        fn from(counters: WorkCounters) -> Self {
            Self {
                seeds: counters.layout_frontier_seeds,
                frontier_measure: counters.layout_frontier_nodes_measure,
                frontier_placement: counters.layout_frontier_nodes_placement,
                contexts: counters.layout_frontier_contexts,
                edges: counters.layout_dependency_edges_visited,
                measured: counters.layout_measure_nodes,
                placed: counters.layout_placement_nodes,
                children_walked: counters.layout_children_measured,
                full_subtrees: counters.intrinsic_measure_full_subtrees,
                constraint_seeds: counters.constraint_change_seeds,
                considered: counters.constraint_dependents_considered,
                remeasured: counters.constraint_dependents_remeasured,
                skipped: counters.constraint_dependents_skipped,
                context_solves: counters.resize_context_solves,
                local_fallbacks: counters.layout_local_subtree_fallbacks,
                full_fallbacks: counters.layout_full_document_fallbacks,
            }
        }
    }

    /// The drags Gates A and B make: out by 120 pixels and back, one each frame.
    fn drag_sizes() -> impl Iterator<Item = f32> {
        (1..=240).map(|step| {
            let out = if step <= 120 { step } else { 240 - step };
            FIRST_SIZE + out as f32
        })
    }

    /// Gates A, B and C over 240 drags of a 10k workspace.
    #[test]
    fn issue264_a_splitter_drag_lays_out_only_what_reads_the_pane_width() {
        // A full layout after each of 240 frames of 10k nodes would cost more
        // than the frames; the cold oracle checks the end, and a 1k workspace
        // runs the per-pass guard below.
        let _unguarded = skip_layout_verify();
        let mut workspace = Workspace::new(5_000, FIRST_SIZE);
        assert!(workspace.world().len() >= 10_000);
        let nodes = workspace.world().document_order(workspace.document);
        let dock = workspace.subtree(workspace.parts.dock);
        let fixed_subtree = workspace.subtree(workspace.parts.fixed_box);
        let mut relayouts = 0;
        let mut first: Option<Cost> = None;
        for size in drag_sizes() {
            let counters = workspace.drag_to(size);
            let cost = Cost::from(counters);
            assert_eq!(
                (cost.local_fallbacks, cost.full_fallbacks),
                (0, 0),
                "{size}: {cost:?}"
            );
            // Gate A: the text that rewraps rewraps from the runs it shaped.
            assert_eq!(
                counters.resize_text_reshapes, 0,
                "{size}: text shaped again"
            );
            relayouts += counters.resize_text_relayouts;
            // Gate A: the dock reads nothing of the split.
            for id in &dock {
                assert!(
                    !admitted(&workspace.context, *id),
                    "{size}: dock node {id:?} laid out"
                );
            }
            // Gates A and C: no fixed box measures, nor anything under the
            // fixed box.
            for id in workspace.parts.fixed.iter().chain(&fixed_subtree) {
                assert!(!workspace.measured(*id), "{size}: fixed {id:?} measured");
            }
            // The second pane narrows as the first widens: it is a constraint
            // seed beside the first, so its children are asked whether they read
            // that, as the first pane's are.
            assert_eq!(cost.constraint_seeds, 2, "{size}: {cost:?}");
            // Gate B: a width-only change; the box that reads only the pane's
            // height keeps its measurement.
            assert!(
                !workspace.measured(workspace.parts.tall),
                "{size}: the height-only box measured"
            );
            // Each drag asks the same children the same question.
            match first {
                None => first = Some(cost),
                Some(first) => assert_eq!(
                    (first.considered, first.remeasured, first.skipped),
                    (cost.considered, cost.remeasured, cost.skipped),
                    "{size}"
                ),
            }
        }
        assert!(relayouts > 0, "the paragraph never rewrapped");
        // Gate A: no node came or went.
        assert_eq!(
            workspace.world().document_order(workspace.document),
            nodes,
            "lifecycle churn"
        );
        let mut cold = Workspace::new(5_000, workspace.size);
        assert_matches_cold(
            &mut workspace.context,
            &mut cold.context,
            workspace.document,
        );
    }

    /// The same drags on a 1k workspace, each pass checked against a full
    /// layout.
    #[test]
    fn issue264_splitter_drags_match_a_full_layout_every_pass() {
        let mut workspace = Workspace::new(100, FIRST_SIZE);
        for size in drag_sizes().step_by(7) {
            workspace.drag_to(size);
        }
        let mut cold = Workspace::new(100, workspace.size);
        assert_matches_cold(
            &mut workspace.context,
            &mut cold.context,
            workspace.document,
        );
    }

    /// Gate B the other way: a height-only change never measures the boxes
    /// that read only the width, and measures the one that reads the height.
    /// The fill-width rows follow their content's height; nothing in them reads
    /// the height they are offered (`world/block_reads.rs`), so their memo keys
    /// drop it.
    #[test]
    fn issue264_a_height_only_resize_leaves_width_readers_alone() {
        let mut workspace = Workspace::fluid(100);
        let counters = workspace.resize(LayoutViewport::new(1200.0, 760.0));
        assert_eq!(counters.layout_full_document_fallbacks, 0);
        for row in &workspace.parts.rows {
            assert!(!workspace.measured(*row), "fill-width row {row:?} measured");
        }
        assert!(
            workspace.measured(workspace.parts.tall),
            "the box that reads the pane's height kept its measurement"
        );
        assert!(!workspace.measured(workspace.parts.paragraph));
        assert_eq!(counters.resize_text_reshapes, 0);
        let mut cold = Workspace::fluid(100);
        cold.resize(LayoutViewport::new(1200.0, 760.0));
        assert_matches_cold(
            &mut workspace.context,
            &mut cold.context,
            workspace.document,
        );
    }

    /// Gate D. A drag costs the same in a workspace of 1k nodes as of 100k, and
    /// asks the same children whether they consume the constraint.
    #[test]
    #[ignore = "100k dense scale gate; run with --release -- --ignored"]
    fn issue264_a_drag_costs_the_same_at_1k_and_100k_nodes() {
        let _unguarded = skip_layout_verify();
        let mut small = Workspace::new(400, FIRST_SIZE);
        let mut huge = Workspace::new(50_000, FIRST_SIZE);
        assert!(small.world().len() <= 1_200);
        assert!(huge.world().len() >= 100_000);
        for size in [FIRST_SIZE + 1.0, FIRST_SIZE + 40.0, FIRST_SIZE - 30.0] {
            let small_cost = Cost::from(small.drag_to(size));
            let huge_cost = Cost::from(huge.drag_to(size));
            assert_eq!(small_cost, huge_cost, "{size}");
            assert!(small_cost.constraint_seeds > 0);
        }
        let mut cold = Workspace::new(50_000, huge.size);
        assert_matches_cold(&mut huge.context, &mut cold.context, huge.document);
    }

    /// A viewport resize goes the same way: the roots, then the boxes that
    /// consume the axis it moved on. The dock stays where it was and is not
    /// laid out, and the cost is the same at 1k and 100k nodes.
    #[test]
    fn issue264_a_viewport_resize_reaches_only_its_consumers() {
        let _unguarded = skip_layout_verify();
        let mut small = Workspace::fluid(400);
        let mut huge = Workspace::fluid(50_000);
        for width in [1201.0, 1260.0, 1100.0] {
            let viewport = LayoutViewport::new(width, 800.0);
            let small_cost = Cost::from(small.resize(viewport));
            let huge_cost = Cost::from(huge.resize(viewport));
            assert_eq!(small_cost, huge_cost, "{width}");
            assert_eq!(
                (small_cost.local_fallbacks, small_cost.full_fallbacks),
                (0, 0)
            );
            assert!(small_cost.constraint_seeds > 0, "{width}: {small_cost:?}");
            for id in small.subtree(small.parts.dock) {
                assert!(
                    !admitted(&small.context, id),
                    "{width}: dock node {id:?} laid out"
                );
            }
        }
        let mut cold = Workspace::fluid(50_000);
        cold.resize(LayoutViewport::new(1100.0, 800.0));
        assert_matches_cold(&mut huge.context, &mut cold.context, huge.document);
    }

    /// A Dock resizes its frames through their flex factors. A factor sizes a
    /// frame along its parent's line only: in a row, the boxes under it that
    /// read the frame's width measure again, and the ones that read only its
    /// height do not.
    #[test]
    fn issue264_a_flex_factor_resize_reaches_only_the_line_axis() {
        let document = DocumentId::new(1).unwrap();
        let (mut b, root) = Builder::new(document, 1);
        let row = b.element(
            root,
            LayoutStyle {
                direction: Some(FlexDirection::Row),
                align_items: AlignSpec::Stretch,
                ..fixed(900.0, 600.0)
            },
        );
        let frame = |grow: f32| LayoutStyle {
            flex_grow: Some(grow),
            direction: Some(FlexDirection::Column),
            ..LayoutStyle::default()
        };
        let left = b.element(row, frame(0.5));
        let right = b.element(row, frame(0.5));
        let mut wide = Vec::new();
        let mut tall = Vec::new();
        for pane in [left, right] {
            let reader = b.element(pane, fill_column());
            b.label(reader, PARAGRAPH);
            wide.push(reader);
            tall.push(b.element(
                pane,
                LayoutStyle {
                    width: Some(LengthSpec::Px(40.0)),
                    height: Some(LengthSpec::Percent(25.0)),
                    ..LayoutStyle::default()
                },
            ));
        }
        let mut context = AppContext::new();
        context.commit_mutations(b.queue).unwrap();
        let mut shaper = bundled_face_shaper();
        product_frame(&mut context, document, viewport(), &mut shaper);
        for step in 1..=20u32 {
            let ratio = 0.5 + step as f32 / 100.0;
            let mut queue = MutationQueue::new();
            queue.set_style(left, styled(frame(ratio)));
            queue.set_style(right, styled(frame(1.0 - ratio)));
            context.commit_mutations(queue).unwrap();
            measure_trace::begin();
            let counters = product_frame(&mut context, document, viewport(), &mut shaper);
            let measured = measure_trace::take();
            assert_eq!(counters.layout_full_document_fallbacks, 0);
            assert_eq!(counters.resize_text_reshapes, 0);
            for id in &tall {
                assert!(
                    !measured.contains(id),
                    "{ratio}: height reader {id:?} measured"
                );
            }
            assert!(
                wide.iter().any(|id| measured.contains(id)),
                "{ratio}: no width reader measured"
            );
        }
    }

    /// A viewport length reads one side of the viewport: a resize along the
    /// other side neither seeds its box nor measures it, and a resize along its
    /// own side does both.
    #[test]
    fn issue264_a_viewport_length_reaches_layout_only_from_its_own_side() {
        use nana_ui_core::ViewportAxis;
        let document = DocumentId::new(1).unwrap();
        let build = || {
            let (mut b, root) = Builder::new(document, 1);
            let page = b.element(root, fill_column());
            let viewport_length = |axis, value| Some(LengthSpec::Viewport { axis, value });
            let wide = b.element(
                page,
                LayoutStyle {
                    width: viewport_length(ViewportAxis::Width, 50.0),
                    height: Some(LengthSpec::Px(20.0)),
                    direction: Some(FlexDirection::Column),
                    ..LayoutStyle::default()
                },
            );
            b.label(wide, PARAGRAPH);
            let tall = b.element(
                page,
                LayoutStyle {
                    width: Some(LengthSpec::Px(20.0)),
                    height: viewport_length(ViewportAxis::Height, 30.0),
                    ..LayoutStyle::default()
                },
            );
            let mut context = AppContext::new();
            context.commit_mutations(b.queue).unwrap();
            (context, wide, tall)
        };
        let (mut context, wide, tall) = build();
        let mut shaper = bundled_face_shaper();
        product_frame(&mut context, document, viewport(), &mut shaper);
        let mut resize = |context: &mut AppContext, width: f32, height: f32| {
            measure_trace::begin();
            let counters = resize_frame(
                context,
                document,
                LayoutViewport::new(width, height),
                &mut shaper,
            );
            (counters, measure_trace::take())
        };
        let box_of = |context: &AppContext, id| context.world().layout_box(id).unwrap();

        // Height only: the vh box, not the vw one.
        let (counters, measured) = resize(&mut context, 1200.0, 700.0);
        assert_eq!(counters.layout_full_document_fallbacks, 0);
        assert!(admitted(&context, tall) && !admitted(&context, wide));
        assert!(measured.contains(&tall) && !measured.contains(&wide));
        assert_eq!(box_of(&context, tall).height, 210.0);

        // Width only: the vw box, not the vh one.
        let frontier_measures = |context: &AppContext, id| {
            context.layout_cause(id).is_some_and(|cause| {
                !cause.pending
                    && cause
                        .invalidation
                        .kind
                        .intersects(InvalidationKind::MEASURE)
            })
        };
        let (counters, measured) = resize(&mut context, 1000.0, 700.0);
        assert_eq!(counters.layout_full_document_fallbacks, 0);
        // The vh box after it may move, so it is placed; it is not measured.
        assert!(admitted(&context, wide) && !frontier_measures(&context, tall));
        assert!(measured.contains(&wide) && !measured.contains(&tall));
        assert_eq!(box_of(&context, wide).width, 500.0);

        let (mut cold, _, _) = build();
        product_frame(
            &mut cold,
            document,
            LayoutViewport::new(1000.0, 700.0),
            &mut bundled_face_shaper(),
        );
        assert_matches_cold(&mut context, &mut cold, document);
    }

    /// A real [`Dock`] dragged through its split ratio, which it projects as
    /// the flex factors of its frames: the boxes under them that read a frame's
    /// width measure again, the ones that read only its height do not.
    #[test]
    fn issue264_a_dock_split_drag_measures_only_the_width_readers() {
        use crate::{Dock, DockAxis, DockNode};
        let document = DocumentId::new(1).unwrap();
        let build = |ratio: f32| {
            let (mut b, root) = Builder::new(document, 1);
            let page = b.element(
                root,
                LayoutStyle {
                    direction: Some(FlexDirection::Row),
                    align_items: AlignSpec::Stretch,
                    ..fixed(900.0, 600.0)
                },
            );
            let mut wide = Vec::new();
            let mut tall = Vec::new();
            let mut contents = Vec::new();
            for _ in 0..2 {
                let content = b.detached(LayoutStyle {
                    width: Some(LengthSpec::Fill),
                    height: Some(LengthSpec::Fill),
                    direction: Some(FlexDirection::Column),
                    ..LayoutStyle::default()
                });
                let reader = b.element(content, fill_column());
                b.label(reader, PARAGRAPH);
                wide.push(reader);
                tall.push(b.element(
                    content,
                    LayoutStyle {
                        width: Some(LengthSpec::Px(40.0)),
                        height: Some(LengthSpec::Percent(25.0)),
                        ..LayoutStyle::default()
                    },
                ));
                contents.push(content);
            }
            let mut context = AppContext::new();
            context.commit_mutations(b.queue).unwrap();
            let dock = context
                .create_component(
                    document,
                    Dock::new(DockNode::split(
                        DockAxis::Horizontal,
                        ratio,
                        DockNode::item("a", Some(contents[0])),
                        DockNode::item("b", Some(contents[1])),
                    )),
                )
                .unwrap();
            context.assemble_dock(dock).unwrap();
            let mut queue = MutationQueue::new();
            queue.insert(page, dock.stable_id(), None);
            context.commit_mutations(queue).unwrap();
            (context, dock, wide, tall)
        };
        let (mut context, dock, wide, tall) = build(0.5);
        let mut shaper = bundled_face_shaper();
        product_frame(&mut context, document, viewport(), &mut shaper);
        let mut ratio = 0.5;
        for step in 1..=20u32 {
            ratio = 0.5 + step as f32 / 100.0;
            context
                .update_component(dock, |dock, _| {
                    assert!(dock.root.set_split_ratio_at(&[], ratio));
                })
                .unwrap();
            measure_trace::begin();
            let counters = product_frame(&mut context, document, viewport(), &mut shaper);
            let measured = measure_trace::take();
            assert_eq!(counters.layout_full_document_fallbacks, 0, "{ratio}");
            assert_eq!(counters.layout_local_subtree_fallbacks, 0, "{ratio}");
            assert_eq!(counters.resize_text_reshapes, 0, "{ratio}");
            for id in &tall {
                assert!(
                    !measured.contains(id),
                    "{ratio}: height reader {id:?} measured"
                );
            }
            assert!(
                wide.iter().all(|id| measured.contains(id)),
                "{ratio}: a width reader kept a stale measurement"
            );
        }
        let (mut cold, _, _, _) = build(ratio);
        product_frame(&mut cold, document, viewport(), &mut bundled_face_shaper());
        assert_matches_cold(&mut context, &mut cold, document);
    }

    /// What sits under a fill-width row whose height follows its content, and
    /// whether a height-only resize must measure the row again because of it.
    #[derive(Debug, Clone, Copy)]
    enum Under {
        PercentHeight,
        FillHeightGrandchild,
        PercentMinHeight,
        ColumnWrap,
        Grid,
        AspectRatio,
        Vertical,
        ContentsAroundPercent,
        AbsolutePercent,
        HiddenPercent,
        FixedAroundPercent,
        Text,
        /// A `vh` child: the row reads nothing itself, but the child's own seed
        /// moves it, and the row measures to take its new height.
        VhChild,
    }

    impl Under {
        const ALL: [Self; 13] = [
            Self::PercentHeight,
            Self::FillHeightGrandchild,
            Self::PercentMinHeight,
            Self::ColumnWrap,
            Self::Grid,
            Self::AspectRatio,
            Self::Vertical,
            Self::ContentsAroundPercent,
            Self::AbsolutePercent,
            Self::HiddenPercent,
            Self::FixedAroundPercent,
            Self::Text,
            Self::VhChild,
        ];

        /// Whether a height-only resize measures the row: its measurement reads
        /// the height it is offered, or a child the resize seeds moved.
        fn measured_on_resize(self) -> bool {
            !matches!(
                self,
                Self::AbsolutePercent | Self::HiddenPercent | Self::FixedAroundPercent | Self::Text
            )
        }

        fn build(self, b: &mut Builder, row: StableNodeId) {
            use nana_ui_core::{DisplaySpec, GridTrack, PositionSpec, WritingModeSpec};
            let percent = |percent| LayoutStyle {
                width: Some(LengthSpec::Px(40.0)),
                height: Some(LengthSpec::Percent(percent)),
                ..LayoutStyle::default()
            };
            match self {
                Self::PercentHeight => {
                    b.element(row, percent(10.0));
                }
                Self::FillHeightGrandchild => {
                    let child = b.element(row, fill_column());
                    b.element(
                        child,
                        LayoutStyle {
                            width: Some(LengthSpec::Px(40.0)),
                            height: Some(LengthSpec::Fill),
                            ..LayoutStyle::default()
                        },
                    );
                }
                Self::PercentMinHeight => {
                    b.element(
                        row,
                        LayoutStyle {
                            width: Some(LengthSpec::Px(40.0)),
                            min_height: Some(LengthSpec::Percent(10.0)),
                            ..LayoutStyle::default()
                        },
                    );
                }
                Self::ColumnWrap => {
                    let wrap = b.element(
                        row,
                        LayoutStyle {
                            flex_wrap: nana_ui_core::FlexWrap::Wrap,
                            direction: Some(FlexDirection::Column),
                            ..LayoutStyle::default()
                        },
                    );
                    for _ in 0..40 {
                        b.element(wrap, fixed(30.0, 30.0));
                    }
                }
                Self::Grid => {
                    let grid = b.element(
                        row,
                        LayoutStyle {
                            display: Some(DisplaySpec::Grid),
                            grid_rows: Some(vec![GridTrack::Fr(1.0), GridTrack::Fr(1.0)]),
                            height: Some(LengthSpec::Percent(10.0)),
                            ..LayoutStyle::default()
                        },
                    );
                    b.element(grid, fixed(30.0, 10.0));
                    b.element(grid, fixed(30.0, 10.0));
                }
                Self::AspectRatio => {
                    b.element(
                        row,
                        LayoutStyle {
                            height: Some(LengthSpec::Percent(10.0)),
                            aspect_ratio: Some(2.0),
                            ..LayoutStyle::default()
                        },
                    );
                }
                Self::Vertical => {
                    let vertical = b.element(
                        row,
                        LayoutStyle {
                            writing_mode: Some(WritingModeSpec::VerticalRl),
                            ..LayoutStyle::default()
                        },
                    );
                    b.label(vertical, PARAGRAPH);
                }
                Self::ContentsAroundPercent => {
                    let contents = b.element(
                        row,
                        LayoutStyle {
                            display: Some(DisplaySpec::Contents),
                            ..LayoutStyle::default()
                        },
                    );
                    b.element(contents, percent(10.0));
                }
                Self::AbsolutePercent => {
                    b.element(
                        row,
                        LayoutStyle {
                            position: PositionSpec::Absolute,
                            ..percent(10.0)
                        },
                    );
                    b.label(row, "label");
                }
                Self::HiddenPercent => {
                    b.element(
                        row,
                        LayoutStyle {
                            hidden: true,
                            ..percent(10.0)
                        },
                    );
                    b.label(row, "label");
                }
                Self::FixedAroundPercent => {
                    let fixed_box = b.element(
                        row,
                        LayoutStyle {
                            direction: Some(FlexDirection::Column),
                            ..fixed(100.0, 50.0)
                        },
                    );
                    b.element(fixed_box, percent(50.0));
                }
                Self::Text => {
                    b.label(row, PARAGRAPH);
                }
                Self::VhChild => {
                    b.element(
                        row,
                        LayoutStyle {
                            width: Some(LengthSpec::Px(40.0)),
                            height: Some(LengthSpec::Viewport {
                                axis: nana_ui_core::ViewportAxis::Height,
                                value: 5.0,
                            }),
                            ..LayoutStyle::default()
                        },
                    );
                }
            }
        }
    }

    /// A page that fills the viewport, a row per case, each laid out and then
    /// resized in height only.
    fn block_read_page(cases: &[Under]) -> (AppContext, DocumentId, Vec<StableNodeId>) {
        let document = DocumentId::new(1).unwrap();
        let (mut b, root) = Builder::new(document, 1);
        let page = b.element(
            root,
            LayoutStyle {
                width: Some(LengthSpec::Fill),
                height: Some(LengthSpec::Fill),
                direction: Some(FlexDirection::Column),
                ..LayoutStyle::default()
            },
        );
        let rows = cases
            .iter()
            .map(|case| {
                let row = b.element(page, fill_column());
                case.build(&mut b, row);
                row
            })
            .collect();
        let mut context = AppContext::new();
        context.commit_mutations(b.queue).unwrap();
        (context, document, rows)
    }

    /// A row whose subtree reads the height it is offered keeps that height in
    /// its memo key and measures again on a height-only resize; one whose
    /// subtree reads none of it -- text, a fixed box around a percentage, an
    /// absolute or hidden child -- does not. Every pass matches a full layout
    /// that keys by the real height, and the end matches a cold layout.
    #[test]
    fn issue264_a_row_keeps_the_offered_height_only_while_its_subtree_reads_it() {
        let (mut context, document, rows) = block_read_page(&Under::ALL);
        let mut shaper = bundled_face_shaper();
        product_frame(&mut context, document, viewport(), &mut shaper);
        for height in [760.0, 700.0, 790.0] {
            measure_trace::begin();
            resize_frame(
                &mut context,
                document,
                LayoutViewport::new(1200.0, height),
                &mut shaper,
            );
            let measured = measure_trace::take();
            for (case, row) in Under::ALL.iter().zip(&rows) {
                assert_eq!(
                    measured.contains(row),
                    case.measured_on_resize(),
                    "{height}: {case:?} row measured {}",
                    measured.contains(row)
                );
            }
        }
        let (mut cold, _, _) = block_read_page(&Under::ALL);
        product_frame(
            &mut cold,
            document,
            LayoutViewport::new(1200.0, 790.0),
            &mut bundled_face_shaper(),
        );
        assert_matches_cold(&mut context, &mut cold, document);
    }

    /// Gate E. The elastic toolbar spans the first pane; from 442 px down to 298
    /// px its deficit falls in its items' padding level. Dragging 320 -> 400 ->
    /// 320 one pixel a frame re-shares the held solve every frame: no envelope is
    /// built, nothing is solved from scratch, nothing is laid out deeper than the
    /// items' own padding, and no text shapes again. Crossing into the gap level
    /// at 450 solves once from scratch; staying there re-shares again.
    #[test]
    fn issue264_e_a_drag_inside_one_envelope_segment_reuses_the_assignment() {
        let mut workspace = Workspace::elastic(100, 320.0);
        workspace.drag_to(321.0);
        let sizes: Vec<f32> = (322..=400)
            .chain((320..400).rev())
            .map(|size| size as f32)
            .collect();
        let mut seen: Vec<(f32, Vec<crate::LayoutBox>)> = Vec::new();
        for (frame, size) in sizes.iter().enumerate() {
            let counters = workspace.drag_to(*size);
            let dynamic = counters.dynamic;
            assert_eq!(
                dynamic.envelope_misses + dynamic.envelope_rebuilds,
                0,
                "{size}"
            );
            assert!(dynamic.envelope_hits > 0, "{size}");
            assert_eq!(dynamic.cold_solves, 0, "{size}: {dynamic:?}");
            assert!(dynamic.incremental_assignments > 0, "{size}");
            assert_eq!(
                (
                    dynamic.deep_expansions,
                    dynamic.child_reflows,
                    dynamic.structural_adaptations,
                    dynamic.budget_fallbacks,
                    dynamic.allocations,
                ),
                (0, 0, 0, 0, 0),
                "{size}"
            );
            assert_eq!(counters.resize_text_reshapes, 0, "{size}");
            assert_eq!(counters.layout_full_document_fallbacks, 0);
            for id in &workspace.parts.elastic_boxes {
                assert!(
                    !workspace.measured(*id),
                    "{size}: fixed box {id:?} measured"
                );
            }
            let boxes: Vec<crate::LayoutBox> = workspace
                .parts
                .elastic_items
                .iter()
                .map(|id| workspace.world().layout_box(*id).unwrap())
                .collect();
            if let Some((_, earlier)) = seen.iter().find(|(width, _)| width == size) {
                assert_eq!(&boxes, earlier, "{size}: not the same on the way back");
            }
            seen.push((*size, boxes));
            if frame % 16 == 0 {
                let mut cold = Workspace::elastic(100, *size);
                assert_matches_cold(
                    &mut workspace.context,
                    &mut cold.context,
                    workspace.document,
                );
            }
        }
        let crossing = workspace.drag_to(450.0).dynamic;
        assert_eq!(crossing.cold_solves, 1, "{crossing:?}");
        let staying = workspace.drag_to(455.0).dynamic;
        assert_eq!(staying.cold_solves, 0, "{staying:?}");
        assert!(staying.incremental_assignments > 0);
        let mut cold = Workspace::elastic(100, 455.0);
        assert_matches_cold(
            &mut workspace.context,
            &mut cold.context,
            workspace.document,
        );
    }
}

mod batch_child_removal {
    //! Issue #389: removing many children of one parent in one commit filters
    //! its child list once in validation and once in apply, and renumbers the
    //! children that stay once, from the first one removed: k of n children cost
    //! O(n + k), not O(n·k). One removal still pays only for the children after
    //! it.

    use std::collections::HashSet;

    use crate::world::reflow_oracle::{
        assert_matches_cold, bundled_face_shaper, column, fixed, node, product_frame, styled,
    };
    use crate::world::{DocumentId, NodeKind, StableNodeId, UiWorld};
    use crate::{AppContext, LayoutViewport, MutationQueue, TextContent};

    const CHILDREN: u64 = 4_000;
    const PARENT: u64 = 2;
    const FIRST_CHILD: u64 = 10;

    fn document() -> DocumentId {
        DocumentId::new(1).unwrap()
    }

    fn element(queue: &mut MutationQueue, parent: StableNodeId, id: StableNodeId) {
        queue.create(id, document(), NodeKind::Element { tag: "div".into() });
        queue.insert(parent, id, None);
    }

    fn child(offset: u64) -> StableNodeId {
        node(FIRST_CHILD + offset)
    }

    /// A document, a parent under it and [`CHILDREN`] children of that parent.
    fn wide_parent() -> UiWorld {
        let mut world = UiWorld::new();
        let mut queue = MutationQueue::new();
        queue.create(node(1), document(), NodeKind::Document);
        element(&mut queue, node(1), node(PARENT));
        for offset in 0..CHILDREN {
            element(&mut queue, node(PARENT), child(offset));
        }
        world.commit(queue).unwrap();
        world
    }

    /// The parent's children, each checked to sit at the index it records.
    fn children(world: &UiWorld) -> Vec<StableNodeId> {
        let children = world.node(node(PARENT)).unwrap().children.to_vec();
        for (index, id) in children.iter().enumerate() {
            assert_eq!(
                world.recorded_child_index(node(PARENT), *id),
                Some(index),
                "{id:?}'s recorded index"
            );
        }
        children
    }

    #[derive(Debug, Clone, Copy)]
    enum Removal {
        Despawn,
        Detach,
        Park,
    }

    fn remove(queue: &mut MutationQueue, removal: Removal, id: StableNodeId) {
        match removal {
            Removal::Despawn => queue.despawn_subtree(id),
            Removal::Detach => queue.detach(id),
            Removal::Park => queue.park_subtree(id),
        }
    }

    /// Each removal shifted, filtered and renumbered the children after it,
    /// in validation and again in apply: clearing a keyed list from the front
    /// paid the whole tail per row. One commit now pays its parent's list a
    /// constant number of times however many of its children leave.
    #[test]
    fn issue389_removing_k_of_n_children_visits_n_plus_k() {
        let n = CHILDREN;
        let shapes: [(&str, Vec<u64>); 4] = [
            ("all, front first", (0..n).collect()),
            ("front half", (0..n / 2).collect()),
            ("every other", (0..n).step_by(2).collect()),
            ("back half, last first", (n / 2..n).rev().collect()),
        ];
        for removal in [Removal::Despawn, Removal::Detach, Removal::Park] {
            for (shape, offsets) in &shapes {
                let mut world = wide_parent();
                let removed = offsets.iter().map(|&o| child(o)).collect::<HashSet<_>>();
                let mut expected = children(&world);
                expected.retain(|id| !removed.contains(id));
                let mut queue = MutationQueue::new();
                for &offset in offsets {
                    remove(&mut queue, removal, child(offset));
                }
                world.child_list_steps = 0;
                world.commit(queue).unwrap();
                let k = offsets.len() as u64;
                assert!(
                    world.child_list_steps <= 2 * (n + k),
                    "{removal:?}, {shape}: {} child-list steps removing {k} of {n}",
                    world.child_list_steps
                );
                assert_eq!(children(&world), expected, "{removal:?}, {shape}");
                for id in &removed {
                    assert_eq!(world.parent_id(*id), None, "{removal:?}, {shape}");
                }
            }
        }
    }

    /// A lone removal is not a run: it shifts and renumbers only the children
    /// after it, and nothing filters the whole list.
    #[test]
    fn issue389_one_removal_pays_only_the_children_after_it() {
        for removal in [Removal::Despawn, Removal::Detach, Removal::Park] {
            let mut world = wide_parent();
            let mut queue = MutationQueue::new();
            remove(&mut queue, removal, child(CHILDREN - 1));
            world.child_list_steps = 0;
            world.commit(queue).unwrap();
            assert_eq!(world.child_list_steps, 0, "{removal:?}, the last child");

            // Ten children after it, each shifted once and renumbered once.
            let mut queue = MutationQueue::new();
            remove(&mut queue, removal, child(CHILDREN - 12));
            world.child_list_steps = 0;
            world.commit(queue).unwrap();
            assert!(
                world.child_list_steps <= 20,
                "{removal:?}: {} steps",
                world.child_list_steps
            );
            assert_eq!(children(&world).len(), CHILDREN as usize - 2);
        }
    }

    /// What one batch does to the parent's list, one edit at a time.
    enum Edit {
        Remove(Removal, u64),
        /// Insert the child at `.0` (a new one past [`CHILDREN`]) before the
        /// child at `.1`, or last.
        Insert(u64, Option<u64>),
    }

    /// Runs of removals broken up by inserts, a reinsertion of a child the
    /// batch detached, a move, a repeated detach and a lone removal leave the
    /// list, and every recorded index, where one edit at a time would.
    #[test]
    fn issue389_removal_runs_between_inserts_keep_order_and_indices() {
        use Edit::{Insert, Remove};
        use Removal::{Despawn, Detach, Park};
        let fresh = CHILDREN;
        let edits = [
            Remove(Despawn, 0),
            Remove(Despawn, 1),
            Remove(Despawn, 2),
            Insert(fresh, Some(5)),
            Remove(Detach, 5),
            Remove(Detach, 6),
            Insert(5, Some(10)),
            Remove(Detach, 10),
            Remove(Despawn, 11),
            Remove(Park, 12),
            Insert(6, None),
            Insert(20, Some(15)),
            Remove(Detach, 40),
            Remove(Detach, 40),
            Remove(Despawn, 41),
            Insert(fresh + 1, Some(3)),
            Remove(Despawn, 3),
            Remove(Despawn, fresh),
            Remove(Park, 50),
        ];
        let mut world = wide_parent();
        let mut expected = children(&world);
        let mut queue = MutationQueue::new();
        for edit in &edits {
            match *edit {
                Remove(removal, offset) => {
                    remove(&mut queue, removal, child(offset));
                    expected.retain(|id| *id != child(offset));
                }
                Insert(offset, before) => {
                    if offset >= fresh {
                        queue.create(
                            child(offset),
                            document(),
                            NodeKind::Element { tag: "div".into() },
                        );
                    }
                    queue.insert(node(PARENT), child(offset), before.map(child));
                    expected.retain(|id| *id != child(offset));
                    let index = before
                        .and_then(|before| expected.iter().position(|id| *id == child(before)))
                        .unwrap_or(expected.len());
                    expected.insert(index, child(offset));
                }
            }
        }
        world.commit(queue).unwrap();
        assert_eq!(children(&world), expected);
    }

    const ROWS: u64 = 200;
    const LABELS: u64 = 100_000;

    fn viewport() -> LayoutViewport {
        LayoutViewport::new(700.0, 600.0)
    }

    fn add_row(queue: &mut MutationQueue, row: u64, before: Option<u64>) {
        queue.create(
            node(row),
            document(),
            NodeKind::Element { tag: "div".into() },
        );
        queue.insert(node(PARENT), node(row), before.map(node));
        queue.set_style(node(row), styled(fixed(200.0, 20.0)));
        let label = node(LABELS + row);
        queue.create(label, document(), NodeKind::Text);
        queue.insert(node(row), label, None);
        queue.set_text(
            label,
            TextContent {
                value: format!("row {row}").into(),
            },
        );
    }

    /// A 400px column of labelled rows, laid out once.
    fn rows(rows: &[u64]) -> AppContext {
        let mut queue = MutationQueue::new();
        queue.create(node(1), document(), NodeKind::Document);
        element(&mut queue, node(1), node(PARENT));
        queue.set_style(node(PARENT), styled(column(Some(400.0))));
        for &row in rows {
            add_row(&mut queue, row, None);
        }
        let mut cx = AppContext::new();
        cx.commit_mutations(queue).unwrap();
        product_frame(&mut cx, document(), viewport(), &mut bundled_face_shaper());
        cx
    }

    /// A bulk removal from the front, a scattered one and a run after an
    /// insert in the middle, in one commit: the next frame lays out as the
    /// rows that stay laid out cold.
    #[test]
    fn issue389_bulk_removal_lays_out_as_cold() {
        let all = (FIRST_CHILD..FIRST_CHILD + ROWS).collect::<Vec<_>>();
        let mut cx = rows(&all);
        let mut queue = MutationQueue::new();
        let mut gone = HashSet::new();
        gone.extend(FIRST_CHILD..FIRST_CHILD + 50);
        gone.extend((100..150).step_by(3));
        for &row in &all {
            if gone.contains(&row) {
                queue.despawn_subtree(node(row));
            }
        }
        let inserted = FIRST_CHILD + ROWS;
        add_row(&mut queue, inserted, Some(160));
        for row in 170..180 {
            gone.insert(row);
            queue.despawn_subtree(node(row));
        }
        cx.commit_mutations(queue).unwrap();
        product_frame(&mut cx, document(), viewport(), &mut bundled_face_shaper());

        let mut stay = all
            .iter()
            .copied()
            .filter(|row| !gone.contains(row))
            .collect::<Vec<_>>();
        let at = stay.iter().position(|row| *row == 160).unwrap();
        stay.insert(at, inserted);
        let mut cold = rows(&stay);
        assert_matches_cold(&mut cx, &mut cold, document());
    }

    /// Validation reads the list a batch has left so far: a child the batch
    /// already detached is no anchor for an insert, as before runs shared a
    /// filter.
    #[test]
    fn issue389_a_detached_child_is_no_insert_anchor_later_in_the_batch() {
        let mut world = wide_parent();
        let before = children(&world);
        let fresh = child(CHILDREN);
        let mut queue = MutationQueue::new();
        queue.detach(child(7));
        queue.detach(child(8));
        queue.create(fresh, document(), NodeKind::Element { tag: "div".into() });
        queue.insert(node(PARENT), fresh, Some(child(8)));
        assert_eq!(
            world.commit(queue),
            Err(crate::UiWorldError::InvalidBefore {
                parent: node(PARENT),
                before: child(8),
            })
        );
        assert_eq!(children(&world), before);
    }
}
