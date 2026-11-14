//! Dynamic overflow adaptation, solver levels, structural gates, and responsive container queries.

#![cfg(test)]

pub(super) mod toolbar_fixture {
    //! Issue #212: a line that solves its overflow takes what its items declare,
    //! cheapest first, and each item resolves its share inside itself.
    //!
    //! One toolbar: a row of six items, each a padded row around a fixed 40x20
    //! box. The toolbar's gaps close to 2 px at the gap price; each item's
    //! padding closes to 4 px a side at the padding price. Six items at 72 px and
    //! five 8 px gaps prefer 472 px.

    use nana_ui_core::dynamic_layout::{
        AdaptationProfile, AxisElasticity, ElasticFloor, ElasticLength, costs,
    };
    use nana_ui_core::{FlexDirection, LayoutStyle, LengthSpec, WorkCounters};

    use crate::world::reflow_oracle::{
        Builder, assert_matches_cold, bundled_face_shaper, fixed, product_frame, styled,
    };
    use crate::world::{DocumentId, StableNodeId};
    use crate::{AppContext, LayoutBox, LayoutViewport, MutationQueue, NanaTextEngineShaper};

    pub(in crate::world::tests) fn gap_profile() -> AdaptationProfile {
        AdaptationProfile {
            inline: AxisElasticity {
                gap: Some(ElasticLength::new(
                    ElasticFloor::Px(2.0),
                    costs::PLACEMENT_GAP,
                )),
                ..AxisElasticity::NONE
            },
            solve_overflow: true,
            ..AdaptationProfile::RIGID
        }
    }

    pub(in crate::world::tests) fn padding_profile() -> AdaptationProfile {
        AdaptationProfile {
            inline: AxisElasticity {
                padding: Some(ElasticLength::new(ElasticFloor::Px(4.0), costs::PADDING)),
                ..AxisElasticity::NONE
            },
            ..AdaptationProfile::RIGID
        }
    }

    /// How the toolbar is built.
    #[derive(Debug, Clone, Copy)]
    pub(in crate::world::tests) struct Bar {
        pub(in crate::world::tests) width: f32,
        /// The toolbar solves its overflow.
        pub(in crate::world::tests) solves: bool,
        /// The items declare their padding.
        pub(in crate::world::tests) declares: bool,
        pub(in crate::world::tests) shrink: Option<f32>,
    }

    impl Bar {
        pub(in crate::world::tests) fn new(width: f32) -> Self {
            Self {
                width,
                solves: true,
                declares: true,
                shrink: None,
            }
        }

        fn toolbar_style(self) -> LayoutStyle {
            LayoutStyle {
                width: Some(LengthSpec::Px(self.width)),
                height: Some(LengthSpec::Px(40.0)),
                direction: Some(FlexDirection::Row),
                gap: Some(LengthSpec::Px(8.0)),
                adaptation: self.solves.then(|| gap_profile().into()),
                ..LayoutStyle::default()
            }
        }

        fn item_style(self) -> LayoutStyle {
            LayoutStyle {
                direction: Some(FlexDirection::Row),
                padding_left: Some(LengthSpec::Px(16.0)),
                padding_right: Some(LengthSpec::Px(16.0)),
                flex_shrink: self.shrink,
                adaptation: self.declares.then(|| padding_profile().into()),
                ..LayoutStyle::default()
            }
        }
    }

    pub(in crate::world::tests) struct Toolbar {
        pub(in crate::world::tests) context: AppContext,
        pub(in crate::world::tests) document: DocumentId,
        shaper: NanaTextEngineShaper,
        pub(in crate::world::tests) bar: StableNodeId,
        pub(in crate::world::tests) items: Vec<StableNodeId>,
        pub(in crate::world::tests) boxes: Vec<StableNodeId>,
        pub(in crate::world::tests) shape: Bar,
    }

    impl Toolbar {
        pub(in crate::world::tests) fn new(shape: Bar) -> Self {
            let document = DocumentId::new(1).unwrap();
            let (mut b, root) = Builder::new(document, 1);
            let bar = b.element(root, shape.toolbar_style());
            let mut items = Vec::new();
            let mut boxes = Vec::new();
            for _ in 0..6 {
                let item = b.element(bar, shape.item_style());
                boxes.push(b.element(item, fixed(40.0, 20.0)));
                items.push(item);
            }
            let mut context = AppContext::new();
            context.commit_mutations(b.queue).unwrap();
            let mut toolbar = Self {
                context,
                document,
                shaper: bundled_face_shaper(),
                bar,
                items,
                boxes,
                shape,
            };
            toolbar.frame();
            toolbar
        }

        pub(in crate::world::tests) fn frame(&mut self) -> WorkCounters {
            product_frame(
                &mut self.context,
                self.document,
                LayoutViewport::new(1200.0, 400.0),
                &mut self.shaper,
            )
        }

        pub(in crate::world::tests) fn resize(&mut self, width: f32) -> WorkCounters {
            self.shape.width = width;
            let mut queue = MutationQueue::new();
            queue.set_style(self.bar, styled(self.shape.toolbar_style()));
            self.context.commit_mutations(queue).unwrap();
            self.frame()
        }

        pub(in crate::world::tests) fn layout(&self, id: StableNodeId) -> LayoutBox {
            self.context.world().layout_box(id).unwrap()
        }

        pub(in crate::world::tests) fn item_boxes(&self) -> Vec<LayoutBox> {
            self.items.iter().map(|id| self.layout(*id)).collect()
        }
    }

    fn near(a: f32, b: f32) -> bool {
        (a - b).abs() < 0.05
    }

    /// A line that fits lays out exactly as one nobody declared anything on,
    /// and asks no item for its envelope.
    #[test]
    fn issue212_a_line_that_fits_is_unchanged() {
        let elastic = Toolbar::new(Bar::new(600.0));
        let rigid = Toolbar::new(Bar {
            solves: false,
            declares: false,
            ..Bar::new(600.0)
        });
        assert_eq!(elastic.item_boxes(), rigid.item_boxes());
        let counters = elastic.context.last_work_counters().dynamic;
        assert_eq!(counters.envelope_queries, 0);
        assert_eq!(counters.solver_runs, 0);
    }

    /// 32 px short: the gaps close first (30 px at the gap price), then the
    /// padding gives the last 2 px. The line fills the toolbar exactly and every
    /// fixed box keeps its size, inside its item's narrower padding.
    #[test]
    fn issue212_an_overflowing_line_closes_gaps_before_padding() {
        let toolbar = Toolbar::new(Bar::new(440.0));
        let items = toolbar.item_boxes();
        let bar = toolbar.layout(toolbar.bar);
        let last = items.last().unwrap();
        assert!(near(last.x + last.width, bar.x + bar.width), "{items:?}");
        for pair in items.windows(2) {
            assert!(
                near(pair[1].x - (pair[0].x + pair[0].width), 2.0),
                "{items:?}"
            );
        }
        let padding_given: f32 = items.iter().map(|item| 72.0 - item.width).sum();
        assert!(near(padding_given, 2.0), "{padding_given}");
        for (item, fixed) in toolbar.items.iter().zip(&toolbar.boxes) {
            let (item, fixed) = (toolbar.layout(*item), toolbar.layout(*fixed));
            assert_eq!((fixed.width, fixed.height), (40.0, 20.0));
            let left = fixed.x - item.x;
            let right = item.x + item.width - (fixed.x + fixed.width);
            assert!(
                near(left, right) && (4.0..16.0).contains(&left),
                "{left} {right}"
            );
        }
    }

    /// Past every declared capacity the solve takes it all; an explicit shrink
    /// factor then clips only what is left, and without one the rest overflows.
    #[test]
    fn issue212_a_shrink_factor_takes_only_what_the_solve_left() {
        let clipped = Toolbar::new(Bar {
            shrink: Some(1.0),
            ..Bar::new(250.0)
        });
        let items = clipped.item_boxes();
        let bar = clipped.layout(clipped.bar);
        let end = items.last().map(|item| item.x + item.width).unwrap();
        assert!(near(end, bar.x + bar.width), "{items:?}");
        let overflowing = Toolbar::new(Bar::new(250.0));
        let items = overflowing.item_boxes();
        for item in &items {
            assert!(near(item.width, 48.0), "{items:?}");
        }
    }

    /// A toolbar that does not solve never reads what its items declare: the
    /// same boxes as a toolbar of items that declare nothing, and no envelope
    /// asked for.
    #[test]
    fn issue212_a_line_that_does_not_solve_ignores_declarations() {
        let declared = Toolbar::new(Bar {
            solves: false,
            ..Bar::new(300.0)
        });
        let plain = Toolbar::new(Bar {
            solves: false,
            declares: false,
            ..Bar::new(300.0)
        });
        assert_eq!(declared.item_boxes(), plain.item_boxes());
        let counters = declared.context.last_work_counters().dynamic;
        assert_eq!(
            (counters.contexts_considered, counters.envelope_queries),
            (0, 0)
        );
    }

    /// Resizing through fitting, gap-only, padding and infeasible widths: every
    /// pass matches a full layout, and the end matches a cold one.
    #[test]
    fn issue212_every_resize_matches_a_full_layout() {
        let mut toolbar = Toolbar::new(Bar::new(600.0));
        for width in [
            480.0, 460.0, 445.0, 400.0, 330.0, 250.0, 300.0, 500.0, 600.0, 320.0,
        ] {
            toolbar.resize(width);
        }
        let mut cold = Toolbar::new(Bar::new(320.0));
        assert_matches_cold(&mut toolbar.context, &mut cold.context, toolbar.document);
    }

    /// Devtools sees what an item declared and what its line gave it: its
    /// envelope's segments and generation, and the part its padding closed up.
    #[test]
    fn issue212_inspect_shows_the_envelope_and_the_assignment() {
        let toolbar = Toolbar::new(Bar::new(330.0));
        let item = toolbar.items[0];
        let dynamic = toolbar.context.inspect(item).unwrap().dynamic.unwrap();
        assert!(dynamic.generation > 0);
        assert_eq!(dynamic.segments.len(), 1);
        assert_eq!(
            dynamic.segments[0].kind,
            nana_ui_core::dynamic_layout::AdjustmentKind::Padding
        );
        let applied = dynamic.applied.unwrap();
        assert!(applied.inline && applied.amount.is_positive());
        assert_eq!(applied.padding, applied.amount);
        let fits = Toolbar::new(Bar::new(600.0));
        assert!(
            fits.context
                .inspect(fits.items[0])
                .unwrap()
                .dynamic
                .is_none()
        );
    }

    /// The built-in strip: a `Toolbar` of `Button`s too wide for its page closes
    /// its gaps and then the buttons' padding, and fits; the same toolbar with
    /// its solving turned off overflows. Buttons in a plain row never compress.
    #[test]
    fn issue212_a_toolbar_of_buttons_closes_up_before_it_overflows() {
        use crate::{Button, Stack, Toolbar};
        use nana_ui_core::dynamic_layout::AdaptationProfile;
        let document = DocumentId::new(1).unwrap();
        let build = |solving: bool| {
            let mut context = AppContext::new();
            let page = context
                .create_component(
                    document,
                    Stack::from_layout(LayoutStyle {
                        width: Some(LengthSpec::Px(440.0)),
                        direction: Some(FlexDirection::Column),
                        ..LayoutStyle::default()
                    }),
                )
                .unwrap();
            let mut toolbar = Toolbar::new();
            if !solving {
                std::sync::Arc::make_mut(&mut toolbar.style.layout).adaptation =
                    Some(AdaptationProfile::RIGID.into());
            }
            let toolbar = context
                .create_detached_component(document, toolbar)
                .unwrap();
            context.append_child(page, toolbar).unwrap();
            let buttons: Vec<_> = (0..6)
                .map(|at| {
                    let button = context
                        .create_detached_component(document, Button::new(format!("Action {at}")))
                        .unwrap();
                    context.append_child(toolbar, button).unwrap();
                    button.stable_id()
                })
                .collect();
            let mut shaper = bundled_face_shaper();
            product_frame(
                &mut context,
                document,
                LayoutViewport::new(800.0, 600.0),
                &mut shaper,
            );
            let bar = context.world().layout_box(toolbar.stable_id()).unwrap();
            let boxes: Vec<LayoutBox> = buttons
                .iter()
                .map(|id| context.world().layout_box(*id).unwrap())
                .collect();
            (context, bar, boxes)
        };
        let (context, bar, solved) = build(true);
        let (_, _, rigid) = build(false);
        let end = |boxes: &[LayoutBox]| boxes.last().map(|last| last.x + last.width).unwrap();
        assert!(
            end(&rigid) > bar.x + bar.width,
            "the rigid toolbar fits: {rigid:?}"
        );
        assert!(
            end(&solved) <= bar.x + bar.width + 0.05,
            "{solved:?} in {bar:?}"
        );
        for (solved, rigid) in solved.iter().zip(&rigid) {
            assert!(solved.width < rigid.width);
        }
        assert!(context.last_work_counters().dynamic.child_resolves >= 6);
    }
}

mod solver_levels {
    //! Issue #213: envelopes and line solves are kept across passes. A resize
    //! whose deficit stays in the cost level the last solve stopped in only
    //! re-shares that level; a resize that crosses a level solves again from
    //! nothing. Either way the answer is the cold one, whatever the history.
    //!
    //! The toolbar of `issue212`: from 442 px down to 298 px the deficit falls
    //! in the items' padding level (the gaps are spent at 442).

    use crate::world::reflow_oracle::assert_matches_cold;
    use crate::world::tests::dynamic_layout::toolbar_fixture::{Bar, Toolbar};

    /// Inside one level, every frame after the first re-shares the held solve:
    /// no envelope is built or rebuilt, no cold solve runs. Going out and back
    /// gives the same boxes at the same width.
    #[test]
    fn issue213_a_resize_inside_one_level_only_re_shares_it() {
        let mut toolbar = Toolbar::new(Bar::new(320.0));
        let mut out = Vec::new();
        for width in (321..=400).map(|width| width as f32) {
            let counters = toolbar.resize(width).dynamic;
            assert_eq!(
                counters.envelope_misses + counters.envelope_rebuilds,
                0,
                "{width}"
            );
            assert!(counters.envelope_hits > 0, "{width}");
            assert_eq!(counters.cold_solves, 0, "{width}: {counters:?}");
            assert!(counters.incremental_assignments > 0, "{width}");
            assert_eq!(counters.budget_fallbacks, 0);
            out.push((width, toolbar.item_boxes()));
        }
        for (width, boxes) in out.iter().rev().skip(1) {
            toolbar.resize(*width);
            assert_eq!(&toolbar.item_boxes(), boxes, "{width} back");
        }
        let mut cold = Toolbar::new(Bar::new(321.0));
        assert_matches_cold(&mut toolbar.context, &mut cold.context, toolbar.document);
    }

    /// Crossing from the padding level into the gap level solves from nothing
    /// once; staying there re-shares again.
    #[test]
    fn issue213_crossing_a_level_solves_once_from_nothing() {
        let mut toolbar = Toolbar::new(Bar::new(400.0));
        let crossing = toolbar.resize(450.0).dynamic;
        assert_eq!(crossing.cold_solves, 1, "{crossing:?}");
        let staying = toolbar.resize(455.0).dynamic;
        assert_eq!(staying.cold_solves, 0, "{staying:?}");
        assert!(staying.incremental_assignments > 0);
        let mut cold = Toolbar::new(Bar::new(455.0));
        assert_matches_cold(&mut toolbar.context, &mut cold.context, toolbar.document);
    }

    /// A width reached from above and from below lays out the same: the held
    /// solve is a hint, never part of the answer.
    #[test]
    fn issue213_the_answer_does_not_depend_on_where_a_resize_came_from() {
        for width in [300.0, 350.0, 441.0, 445.0, 470.0] {
            let mut from_above = Toolbar::new(Bar::new(600.0));
            from_above.resize(width);
            let mut from_below = Toolbar::new(Bar::new(200.0));
            from_below.resize(width);
            let fresh = Toolbar::new(Bar::new(width));
            assert_eq!(
                from_above.item_boxes(),
                fresh.item_boxes(),
                "{width} from above"
            );
            assert_eq!(
                from_below.item_boxes(),
                fresh.item_boxes(),
                "{width} from below"
            );
        }
    }
}

mod structural_gates {
    //! Issue #214: the structural gates of Dynamic Layout. Counters, not time.
    //!
    //! - A: a static frame and a paint-only change run no solve and ask no
    //!   envelope; a container that does not solve never reads a declaration.
    //! - B: a thousand elastic items short by what their padding covers: one
    //!   segment read per item, nothing deeper, linear from 1k to 2k, and a
    //!   resize inside the level allocates nothing.
    //! - C: a nested elastic tree, four children a level: every node resolves
    //!   at most once a pass and its envelope is built once; work grows with
    //!   the nodes, not with branching to the depth.
    //! - E: 240 resizes across three levels: after warm-up no envelope is built,
    //!   and a solve runs from scratch only where the level changes.
    //! - F: a line more crowded than the solver's budget falls back once per
    //!   solve, the same way every time.
    //! - G: 500 frames of resizes, item churn and profile edits leave the kept
    //!   envelopes and solves what the tree gives them.
    //! - D, the break-heavy text gate, is in `nana-text`'s `line_decision_gate`.

    use nana_ui_core::dynamic_layout::{
        AdaptationProfile, AxisElasticity, ElasticFloor, ElasticLength, costs,
    };
    use nana_ui_core::{FlexDirection, LayoutStyle, LengthSpec, SemanticColorRole, WorkCounters};

    use crate::layout_engine::measure_trace;
    use crate::layout_engine::verify::skip_layout_verify;
    use crate::world::reflow_oracle::{
        Builder, assert_matches_cold, bundled_face_shaper, fixed, product_frame, styled,
    };
    use crate::world::tests::dynamic_layout::toolbar_fixture::{Bar, Toolbar, padding_profile};
    use crate::world::{DocumentId, StableNodeId};
    use crate::{AppContext, LayoutViewport, MutationQueue, NanaTextEngineShaper};

    fn viewport() -> LayoutViewport {
        LayoutViewport::new(100_000.0, 400.0)
    }

    /// A row of `count` padded leaves, `width` wide, solving its overflow.
    struct Row {
        context: AppContext,
        document: DocumentId,
        shaper: NanaTextEngineShaper,
        row: StableNodeId,
        leaves: Vec<StableNodeId>,
        width: f32,
    }

    impl Row {
        fn new(count: usize, width: f32) -> Self {
            let document = DocumentId::new(1).unwrap();
            let (mut b, root) = Builder::new(document, 1);
            let row = b.element(root, row_style(width));
            let leaves = (0..count)
                .map(|_| {
                    let leaf = b.element(row, leaf_style());
                    b.element(leaf, fixed(4.0, 4.0));
                    leaf
                })
                .collect();
            let mut context = AppContext::new();
            context.commit_mutations(b.queue).unwrap();
            let mut row = Self {
                context,
                document,
                shaper: bundled_face_shaper(),
                row,
                leaves,
                width,
            };
            row.frame();
            row
        }

        fn frame(&mut self) -> WorkCounters {
            product_frame(
                &mut self.context,
                self.document,
                viewport(),
                &mut self.shaper,
            )
        }

        fn resize(&mut self, width: f32) -> WorkCounters {
            self.width = width;
            let mut queue = MutationQueue::new();
            queue.set_style(self.row, styled(row_style(width)));
            self.context.commit_mutations(queue).unwrap();
            self.frame()
        }
    }

    fn row_style(width: f32) -> LayoutStyle {
        LayoutStyle {
            width: Some(LengthSpec::Px(width)),
            height: Some(LengthSpec::Px(20.0)),
            direction: Some(FlexDirection::Row),
            adaptation: Some(
                AdaptationProfile {
                    solve_overflow: true,
                    ..AdaptationProfile::RIGID
                }
                .into(),
            ),
            ..LayoutStyle::default()
        }
    }

    /// 36 px preferred (16 + 4 + 16), 24 px of it padding that may close up.
    fn leaf_style() -> LayoutStyle {
        LayoutStyle {
            direction: Some(FlexDirection::Row),
            padding_left: Some(LengthSpec::Px(16.0)),
            padding_right: Some(LengthSpec::Px(16.0)),
            adaptation: Some(padding_profile().into()),
            ..LayoutStyle::default()
        }
    }

    /// Gate A.
    #[test]
    fn issue214_a_static_frame_and_a_paint_change_solve_nothing() {
        let mut toolbar = Toolbar::new(Bar::new(330.0));
        let first = toolbar.context.last_work_counters().dynamic;
        assert!(first.solver_runs > 0);
        let applied_before = toolbar.item_boxes();
        // A static frame has no work at all, so no layout and no solve.
        assert!(toolbar.context.take_system_work().is_empty());
        // A paint-only edit of every item.
        let mut queue = MutationQueue::new();
        for item in &toolbar.items {
            let mut style = toolbar.context.world().node_style(*item).unwrap().clone();
            style.background = Some(SemanticColorRole::Accent);
            queue.set_style(*item, style);
        }
        toolbar.context.commit_mutations(queue).unwrap();
        // Paint reaches no layout: no seed, so no solve, no envelope asked for.
        let work = toolbar.context.take_system_work();
        assert!(
            work.layout_frontier_seeds.is_empty(),
            "{:?}",
            work.layout_frontier_seeds
        );
        assert!(
            toolbar
                .context
                .take_layout_frontier_seeds(toolbar.document)
                .is_empty()
        );
        assert_eq!(toolbar.item_boxes(), applied_before);
    }

    /// Gate B.
    #[test]
    fn issue214_b_a_cheap_deficit_reads_each_item_once() {
        let _unguarded = skip_layout_verify();
        let mut costs = Vec::new();
        for count in [1_000usize, 2_000] {
            // Short by half of what the padding covers.
            let mut row = Row::new(count, count as f32 * 24.0);
            let first = row.context.last_work_counters().dynamic;
            assert_eq!(first.cost_segments_visited, count, "{count}");
            assert_eq!(
                (
                    first.deep_expansions,
                    first.child_reflows,
                    first.budget_fallbacks
                ),
                (0, 0, 0)
            );
            assert_eq!(first.child_resolves, count);
            // A resize inside the level: re-shared, nothing allocated, and no
            // leaf's content measured again.
            measure_trace::begin();
            let next = row.resize(count as f32 * 24.0 + 7.0).dynamic;
            let measured = measure_trace::take();
            assert_eq!((next.cold_solves, next.allocations), (0, 0), "{next:?}");
            assert!(next.incremental_assignments > 0);
            for leaf in &row.leaves {
                let content = row.context.world().node(*leaf).unwrap().children[0];
                assert!(
                    !measured.contains(&content),
                    "{count}: leaf content measured"
                );
            }
            costs.push(first);
        }
        let ratio = |a: usize, b: usize| b as f64 / a as f64;
        assert!(
            ratio(
                costs[0].cost_segments_visited,
                costs[1].cost_segments_visited
            ) <= 2.05
        );
        assert!(ratio(costs[0].child_resolves, costs[1].child_resolves) <= 2.05);
    }

    /// An elastic tree, `branching` children a level, `depth` levels of rows
    /// over padded leaves: every row passes its children's elasticity up and
    /// solves its own line.
    fn nested(depth: u32, branching: usize, width: f32) -> (AppContext, DocumentId, usize) {
        fn row(
            b: &mut Builder,
            parent: StableNodeId,
            depth: u32,
            branching: usize,
            nodes: &mut usize,
        ) {
            *nodes += 1;
            if depth == 0 {
                let leaf = b.element(
                    parent,
                    LayoutStyle {
                        direction: Some(FlexDirection::Row),
                        padding_left: Some(LengthSpec::Px(8.0)),
                        padding_right: Some(LengthSpec::Px(8.0)),
                        adaptation: Some(padding_profile().into()),
                        ..LayoutStyle::default()
                    },
                );
                b.element(leaf, fixed(4.0, 4.0));
                return;
            }
            let elastic = AdaptationProfile {
                inline: AxisElasticity {
                    padding: Some(ElasticLength::new(ElasticFloor::Px(1.0), costs::PADDING)),
                    gap: Some(ElasticLength::new(
                        ElasticFloor::Px(1.0),
                        costs::PLACEMENT_GAP,
                    )),
                    content_gap: None,
                },
                aggregate_children: true,
                ..AdaptationProfile::RIGID
            };
            let node = b.element(
                parent,
                LayoutStyle {
                    direction: Some(FlexDirection::Row),
                    padding_left: Some(LengthSpec::Px(4.0)),
                    padding_right: Some(LengthSpec::Px(4.0)),
                    gap: Some(LengthSpec::Px(4.0)),
                    adaptation: Some(elastic.into()),
                    ..LayoutStyle::default()
                },
            );
            for _ in 0..branching {
                row(b, node, depth - 1, branching, nodes);
            }
        }
        let document = DocumentId::new(1).unwrap();
        let (mut b, root) = Builder::new(document, 1);
        let top = b.element(root, row_style(width));
        let mut nodes = 0;
        for _ in 0..branching {
            row(&mut b, top, depth, branching, &mut nodes);
        }
        let mut context = AppContext::new();
        context.commit_mutations(b.queue).unwrap();
        (context, document, nodes)
    }

    /// Gate C.
    #[test]
    fn issue214_c_a_nested_tree_resolves_each_node_once() {
        let mut work = Vec::new();
        for depth in [2u32, 3] {
            let (mut context, document, elastic_nodes) = nested(depth, 4, 200.0);
            let mut shaper = bundled_face_shaper();
            let first = product_frame(&mut context, document, viewport(), &mut shaper).dynamic;
            assert!(first.child_resolves > 0);
            assert!(first.child_resolves <= elastic_nodes, "{depth}: {first:?}");
            assert!(first.envelope_misses <= elastic_nodes, "{depth}: {first:?}");
            // Resize the top row a little: nothing is built again.
            let mut queue = MutationQueue::new();
            let top = context
                .world()
                .node(StableNodeId::new(1).unwrap())
                .unwrap()
                .children[0];
            queue.set_style(top, styled(row_style(203.0)));
            context.commit_mutations(queue).unwrap();
            let next = product_frame(&mut context, document, viewport(), &mut shaper).dynamic;
            assert_eq!(
                next.envelope_misses + next.envelope_rebuilds,
                0,
                "{depth}: {next:?}"
            );
            assert!(next.child_resolves <= elastic_nodes);
            let mut cold = nested(depth, 4, 203.0).0;
            product_frame(&mut cold, document, viewport(), &mut bundled_face_shaper());
            assert_matches_cold(&mut context, &mut cold, document);
            work.push((
                elastic_nodes,
                first.child_resolves + first.cost_segments_visited,
            ));
        }
        let node_ratio = work[1].0 as f64 / work[0].0 as f64;
        let work_ratio = work[1].1 as f64 / work[0].1 as f64;
        assert!(work_ratio <= node_ratio * 1.05, "{work:?}");
    }

    /// Gate E.
    #[test]
    fn issue214_e_a_resize_storm_builds_nothing_and_solves_cold_only_across_levels() {
        // The toolbar's levels: padding below 442 px, gaps below 472 px.
        let level = |width: f32| {
            if width >= 472.0 {
                0
            } else if width >= 442.0 {
                1
            } else {
                2
            }
        };
        let mut toolbar = Toolbar::new(Bar::new(301.0));
        let widths: Vec<f32> = (0..240)
            .map(|frame| {
                let step = frame % 120;
                let out = if step < 60 { step } else { 120 - step };
                300.0 + out as f32 * 3.0
            })
            .collect();
        let mut previous = level(300.0);
        for (frame, width) in widths.iter().enumerate() {
            let dynamic = toolbar.resize(*width).dynamic;
            assert_eq!(
                dynamic.envelope_misses + dynamic.envelope_rebuilds,
                0,
                "{width}"
            );
            let now = level(*width);
            let expected_cold = usize::from(now != 0 && now != previous);
            assert_eq!(
                dynamic.cold_solves, expected_cold,
                "{frame} {width}: {dynamic:?}"
            );
            previous = now;
            if frame % 16 == 0 {
                let mut cold = Toolbar::new(Bar::new(*width));
                assert_matches_cold(&mut toolbar.context, &mut cold.context, toolbar.document);
            }
        }
    }

    /// Gate F.
    #[test]
    fn issue214_f_a_line_past_the_budget_falls_back_the_same_way_every_time() {
        let _unguarded = skip_layout_verify();
        let count = crate::layout_engine::dynamic_line_max_states() + 904;
        let width = count as f32 * 30.0;
        let mut runs = Vec::new();
        for _ in 0..2 {
            let row = Row::new(count, width);
            let dynamic = row.context.last_work_counters().dynamic;
            assert_eq!(dynamic.budget_fallbacks, 1, "{dynamic:?}");
            assert!(dynamic.cost_segments_visited <= count);
            runs.push(
                row.leaves
                    .iter()
                    .map(|leaf| row.context.world().layout_box(*leaf).unwrap())
                    .collect::<Vec<_>>(),
            );
        }
        assert_eq!(runs[0], runs[1]);
        let mut row = Row::new(count, width);
        let mut cold = Row::new(count, width);
        assert_matches_cold(&mut row.context, &mut cold.context, row.document);
    }

    /// Gate G.
    #[test]
    fn issue214_g_kept_envelopes_and_solves_follow_the_tree() {
        let mut toolbar = Toolbar::new(Bar::new(330.0));
        let spare = toolbar.items[5];
        let mut at_ten = None;
        for frame in 0..500u32 {
            let width = 300.0 + (frame % 40) as f32 * 4.0;
            if frame % 50 == 25 {
                let mut queue = MutationQueue::new();
                queue.detach(spare);
                toolbar.context.commit_mutations(queue).unwrap();
                toolbar.frame();
                let mut queue = MutationQueue::new();
                queue.insert(toolbar.bar, spare, None);
                toolbar.context.commit_mutations(queue).unwrap();
            }
            if frame % 100 == 50 {
                let mut style = toolbar.context.world().node_style(spare).unwrap().clone();
                let mut profile = padding_profile();
                profile.inline.padding = Some(ElasticLength::new(
                    ElasticFloor::Px(2.0 + (frame / 100) as f32),
                    costs::PADDING,
                ));
                std::sync::Arc::make_mut(&mut style.layout).adaptation = Some(profile.into());
                let mut queue = MutationQueue::new();
                queue.set_style(spare, style);
                toolbar.context.commit_mutations(queue).unwrap();
            }
            toolbar.resize(width);
            let footprint = toolbar.context.retained_layout_footprint(toolbar.document);
            let nodes = toolbar.context.world().len();
            assert!(footprint.envelopes <= nodes * 2);
            assert!(footprint.line_solve_entries <= toolbar.items.len() * footprint.line_solves);
            if frame == 10 {
                at_ten = Some(footprint);
            }
            if frame == 490 {
                let at_ten = at_ten.unwrap();
                assert_eq!(
                    (
                        footprint.envelopes,
                        footprint.line_solves,
                        footprint.applied_adjustments
                    ),
                    (
                        at_ten.envelopes,
                        at_ten.line_solves,
                        at_ten.applied_adjustments
                    )
                );
            }
        }
    }
}

pub(super) mod responsive_queries {
    //! Issue #265: responsive rules read their container's size through an
    //! index and move layout only by what their variants change.
    //!
    //! A page of filler groups ends in a panel 4000 px tall whose width the test
    //! sets. A hundred rows sit in the panel, each 200x24 and each following a
    //! rule on the panel's inline size: ten turn 36 px tall below 500 px, the
    //! other ninety below 200 px.
    //!
    //! - Gate A: the panel's width moves: the hundred rules are evaluated and no
    //!   other node is, the same at 10k and at 100k nodes.
    //! - Gate B: 240 widths in one bucket: every rule is evaluated, none changes,
    //!   and nothing is laid out for them.
    //! - Gate C: the width crosses 500: the ten rules that break there change
    //!   and seed one invalidation each; the ninety fixed rows measure nothing.
    //! - Gate D: eight containers nested, each with fifty fixed leaves, each
    //!   tightening its padding below 400 px of its parent: a resize evaluates
    //!   the seven rules and no leaf, converges in at most four rounds, needs no
    //!   fallback, and matches the same page laid out cold.
    //! - Gate E: the index holds what was registered, and a frame's history is
    //!   gone by the next frame.
    //!
    //! A rule whose variant moves its own container back across its breakpoint
    //! settles in its frame: it keeps the bucket it reached, deterministically.

    use std::sync::Arc;

    use nana_ui_core::{ContainerType, FlexDirection, LayoutStyle, LengthSpec, WorkCounters};

    use crate::layout_engine::verify::skip_layout_verify;
    use crate::world::reflow_oracle::{
        assert_matches_cold, bundled_face_shaper, node, product_frame, styled,
    };
    use crate::world::{DocumentId, NodeKind, StableNodeId};
    use crate::{
        AppContext, LayoutViewport, MutationQueue, NanaTextEngineShaper, NodeStyle, ResponsiveAxis,
        ResponsiveContainer, ResponsiveRule, StyleVariant,
    };

    pub(in crate::world::tests) const RULES: u64 = 100;
    const PANEL: u64 = 10_000_000;

    fn viewport() -> LayoutViewport {
        LayoutViewport::new(1200.0, 800.0)
    }

    fn element(
        queue: &mut MutationQueue,
        document: DocumentId,
        parent: StableNodeId,
        id: StableNodeId,
        style: NodeStyle,
    ) {
        queue.create(id, document, NodeKind::Element { tag: "div".into() });
        queue.insert(parent, id, None);
        queue.set_style(id, style);
    }

    fn column(width: Option<f32>, height: Option<f32>) -> LayoutStyle {
        LayoutStyle {
            width: width.map(LengthSpec::Px),
            height: height.map(LengthSpec::Px),
            direction: Some(FlexDirection::Column),
            flex_shrink: Some(0.0),
            ..LayoutStyle::default()
        }
    }

    fn fixed(width: f32, height: f32) -> LayoutStyle {
        LayoutStyle {
            width: Some(LengthSpec::Px(width)),
            height: Some(LengthSpec::Px(height)),
            flex_shrink: Some(0.0),
            ..LayoutStyle::default()
        }
    }

    /// The page under the document, and groups of a hundred fixed rows filling
    /// it to about `nodes` nodes, numbered from `first`. The page holds groups,
    /// not a hundred thousand children of its own.
    fn page_with_filler(queue: &mut MutationQueue, document: DocumentId, nodes: u64, first: u64) {
        queue.create(node(1), document, NodeKind::Document);
        element(
            queue,
            document,
            node(1),
            node(2),
            styled(column(Some(1200.0), None)),
        );
        let groups = (nodes.saturating_sub(RULES + 3) / 100).max(1);
        let mut next = first;
        for _ in 0..groups {
            let group = node(next);
            next += 1;
            element(queue, document, node(2), group, styled(column(None, None)));
            for _ in 0..99 {
                element(
                    queue,
                    document,
                    group,
                    node(next),
                    styled(fixed(100.0, 4.0)),
                );
                next += 1;
            }
        }
    }

    /// Row `index` of the panel.
    pub(in crate::world::tests) fn row(index: u64) -> StableNodeId {
        node(PANEL + 1 + index)
    }

    /// Where row `index` breaks: every tenth row at 500, the rest at 200.
    pub(in crate::world::tests) fn breakpoint(index: u64) -> f32 {
        if index.is_multiple_of(10) {
            500.0
        } else {
            200.0
        }
    }

    fn row_rule(index: u64) -> Arc<ResponsiveRule> {
        Arc::new(
            ResponsiveRule::new(ResponsiveContainer::Parent, ResponsiveAxis::Inline)
                .below(breakpoint(index), |layout| {
                    layout.height = Some(LengthSpec::Px(36.0))
                }),
        )
    }

    /// What responsive rules cost one frame.
    #[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
    pub(in crate::world::tests) struct Queries {
        pub(in crate::world::tests) size_changes: usize,
        pub(in crate::world::tests) evaluated: usize,
        pub(in crate::world::tests) changed: usize,
        pub(in crate::world::tests) unchanged: usize,
        pub(in crate::world::tests) downstream: usize,
        pub(in crate::world::tests) rounds: usize,
        pub(in crate::world::tests) fallbacks: usize,
    }

    impl From<WorkCounters> for Queries {
        fn from(counters: WorkCounters) -> Self {
            Self {
                size_changes: counters.container_query_size_changes,
                evaluated: counters.container_query_rules_evaluated,
                changed: counters.container_query_results_changed,
                unchanged: counters.container_query_results_unchanged,
                downstream: counters.container_query_downstream_invalidations,
                rounds: counters.container_query_convergence_rounds,
                fallbacks: counters.container_query_cycle_fallbacks,
            }
        }
    }

    /// A page of about `nodes` nodes ending in a box whose width the test sets.
    pub(in crate::world::tests) struct Panel {
        pub(in crate::world::tests) context: AppContext,
        document: DocumentId,
        shaper: NanaTextEngineShaper,
        guarded: bool,
        /// The box [`Self::resize`] restyles, and its layout at a width.
        resized: StableNodeId,
        layout_at: fn(f32) -> LayoutStyle,
    }

    impl Panel {
        /// The page ending in the panel of a hundred rows.
        pub(in crate::world::tests) fn new(nodes: u64, width: f32, guarded: bool) -> Self {
            let document = DocumentId::new(1).unwrap();
            let mut queue = MutationQueue::new();
            page_with_filler(&mut queue, document, nodes, 10);
            // The panel ends the page and keeps its height: a row that grows
            // moves nothing outside it.
            element(
                &mut queue,
                document,
                node(2),
                node(PANEL),
                styled(column(Some(width), Some(4000.0))),
            );
            for index in 0..RULES {
                element(
                    &mut queue,
                    document,
                    node(PANEL),
                    row(index),
                    styled(fixed(200.0, 24.0)),
                );
                queue.set_responsive(row(index), Some(row_rule(index)));
            }
            Self::laid_out(document, queue, guarded, node(PANEL), |width| {
                column(Some(width), Some(4000.0))
            })
        }

        /// `queue` committed to a new context and laid out once.
        fn laid_out(
            document: DocumentId,
            queue: MutationQueue,
            guarded: bool,
            resized: StableNodeId,
            layout_at: fn(f32) -> LayoutStyle,
        ) -> Self {
            let mut context = AppContext::new();
            context.compat_world_mut().commit(queue).unwrap();
            let mut panel = Self {
                context,
                document,
                shaper: bundled_face_shaper(),
                guarded,
                resized,
                layout_at,
            };
            panel.frame();
            panel
        }

        pub(in crate::world::tests) fn frame(&mut self) -> WorkCounters {
            let _unguarded = (!self.guarded).then(skip_layout_verify);
            product_frame(
                &mut self.context,
                self.document,
                viewport(),
                &mut self.shaper,
            )
        }

        /// One frame at `width`.
        pub(in crate::world::tests) fn resize(&mut self, width: f32) -> WorkCounters {
            let mut queue = MutationQueue::new();
            queue.set_style(self.resized, styled((self.layout_at)(width)));
            self.context.commit_mutations(queue).unwrap();
            self.frame()
        }

        fn height(&self, index: u64) -> f32 {
            self.context.world().layout_box(row(index)).unwrap().height
        }

        fn footprint(&self) -> (usize, usize, usize, usize) {
            self.context.world().responsive.footprint()
        }
    }

    #[test]
    fn issue265_gate_a_a_container_resize_evaluates_its_rules_and_nothing_else() {
        let mut costs = Vec::new();
        for nodes in [10_000, 100_000] {
            let mut panel = Panel::new(nodes, 800.0, false);
            for index in 0..RULES {
                assert_eq!(panel.height(index), 24.0, "{nodes}: row {index}");
            }
            let counters = panel.resize(790.0);
            let queries = Queries::from(counters);
            // The panel's inline size moved; its hundred rules read it.
            assert_eq!(queries.size_changes, 1, "{nodes}: {queries:?}");
            assert_eq!(queries.evaluated, RULES as usize, "{nodes}: {queries:?}");
            assert_eq!(queries.unchanged, RULES as usize);
            assert_eq!(queries.changed, 0);
            assert_eq!(counters.layout_full_document_fallbacks, 0);
            costs.push((
                queries,
                counters.layout_frontier_seeds,
                counters.layout_frontier_nodes_measure,
                counters.layout_frontier_nodes_placement,
                counters.layout_measure_nodes,
                counters.layout_placement_nodes,
                counters.layout_dependency_edges_visited,
            ));
        }
        // Ninety thousand more nodes outside the panel cost nothing.
        assert_eq!(costs[0], costs[1]);
    }

    #[test]
    fn issue265_gate_b_a_resize_storm_in_one_bucket_changes_nothing() {
        let mut panel = Panel::new(1_000, 800.0, true);
        let footprint = panel.footprint();
        let mut total = Queries::default();
        for step in 0..240u32 {
            // Every width at or above 500: one bucket for every rule.
            let width = 510.0 + ((step * 37) % 290) as f32;
            let counters = panel.resize(width);
            let queries = Queries::from(counters);
            assert_eq!(queries.changed, 0, "step {step}: {queries:?}");
            assert_eq!(queries.downstream, 0, "step {step}: {queries:?}");
            total.evaluated += queries.evaluated;
            total.size_changes += queries.size_changes;
            // The panel lays out at its new width; no row measures.
            assert!(
                counters.layout_measure_nodes <= 1,
                "step {step}: {} measured",
                counters.layout_measure_nodes
            );
        }
        assert_eq!(total.size_changes, 240);
        assert_eq!(total.evaluated, 240 * RULES as usize);
        // Gate E: the index is what was registered, not what the frames did.
        assert_eq!(panel.footprint(), footprint);
        assert_eq!(footprint, (RULES as usize, RULES as usize, 1, 0));
    }

    #[test]
    fn issue265_gate_c_one_breakpoint_changes_only_the_rules_it_breaks() {
        let mut panel = Panel::new(1_000, 520.0, true);
        let counters = panel.resize(480.0);
        let queries = Queries::from(counters);
        let breaking = (0..RULES)
            .filter(|index| breakpoint(*index) == 500.0)
            .count();
        assert_eq!(breaking, 10);
        assert_eq!(queries.evaluated, RULES as usize, "{queries:?}");
        assert_eq!(queries.changed, breaking, "{queries:?}");
        assert_eq!(queries.unchanged, RULES as usize - breaking);
        assert_eq!(queries.downstream, breaking, "one seed per variant");
        assert_eq!(queries.rounds, 1);
        assert_eq!(queries.fallbacks, 0);
        for index in 0..RULES {
            let expected = if breakpoint(index) == 500.0 {
                36.0
            } else {
                24.0
            };
            assert_eq!(panel.height(index), expected, "row {index}");
        }
        // The ten rows measure; the ninety fixed ones below them only move.
        assert!(
            counters.layout_measure_nodes <= breaking + 2,
            "{} measured",
            counters.layout_measure_nodes
        );
        // The frontier is the ten rows and the path from the panel to the root:
        // the panel, the page and the document.
        assert!(
            counters.layout_frontier_nodes_measure <= breaking + 3,
            "{} in the measure frontier",
            counters.layout_frontier_nodes_measure
        );
        // History of the frame that changed: the ten buckets they left. The
        // next frame starts over.
        assert_eq!(panel.footprint().3, breaking);
        let counters = panel.resize(470.0);
        assert_eq!(Queries::from(counters).changed, 0);
        assert_eq!(panel.footprint().3, 0);

        // And back.
        let queries = Queries::from(panel.resize(520.0));
        assert_eq!(queries.changed, breaking);
        for index in 0..RULES {
            assert_eq!(panel.height(index), 24.0, "row {index}");
        }
    }

    const DEPTH: u64 = 8;
    const LEAVES: u64 = 50;

    fn level(depth: u64) -> StableNodeId {
        node(100 + depth)
    }

    fn padded(width: LengthSpec, padding: f32) -> LayoutStyle {
        LayoutStyle {
            width: Some(width),
            padding: Some(LengthSpec::Px(padding)),
            direction: Some(FlexDirection::Column),
            flex_shrink: Some(0.0),
            ..LayoutStyle::default()
        }
    }

    /// The outermost container: `width` wide, its height its own.
    fn outer(width: f32) -> LayoutStyle {
        LayoutStyle {
            height: Some(LengthSpec::Px(4000.0)),
            ..padded(LengthSpec::Px(width), 8.0)
        }
    }

    impl Panel {
        /// Eight containers nested at the end of a page of about `nodes` nodes,
        /// each filling its parent and holding the next and fifty fixed leaves;
        /// container `i > 0` tightens its padding below 400 px of its parent. The
        /// outermost keeps its height, so nothing outside it moves.
        pub(in crate::world::tests) fn nest(width: f32, nodes: u64, guarded: bool) -> Self {
            let document = DocumentId::new(1).unwrap();
            let mut queue = MutationQueue::new();
            page_with_filler(&mut queue, document, nodes, 20_000_000);
            element(
                &mut queue,
                document,
                node(2),
                level(0),
                styled(outer(width)),
            );
            let mut leaf = 1_000u64;
            for depth in 0..DEPTH {
                if depth > 0 {
                    element(
                        &mut queue,
                        document,
                        level(depth - 1),
                        level(depth),
                        styled(padded(LengthSpec::Fill, 8.0)),
                    );
                    queue.set_responsive(
                        level(depth),
                        Some(Arc::new(
                            ResponsiveRule::new(
                                ResponsiveContainer::Parent,
                                ResponsiveAxis::Inline,
                            )
                            .below(400.0, |layout| layout.padding = Some(LengthSpec::Px(2.0))),
                        )),
                    );
                }
                for _ in 0..LEAVES {
                    element(
                        &mut queue,
                        document,
                        level(depth),
                        node(leaf),
                        styled(fixed(20.0, 4.0)),
                    );
                    leaf += 1;
                }
            }
            Self::laid_out(document, queue, guarded, level(0), outer)
        }

        fn buckets(&self) -> Vec<Option<usize>> {
            (1..DEPTH)
                .map(|depth| self.context.world().responsive_bucket(level(depth)))
                .collect()
        }
    }

    #[test]
    fn issue265_gate_d_nested_containers_converge_along_their_chain() {
        let mut nest = Panel::nest(800.0, 1_000, true);
        assert_eq!(nest.buckets(), vec![Some(1); 7]);
        let counters = nest.resize(300.0);
        let queries = Queries::from(counters);
        // Seven rules, one per level; no leaf has one to evaluate.
        assert!(queries.evaluated >= 7, "{queries:?}");
        assert!(queries.evaluated <= 7 * 4, "{queries:?}");
        assert_eq!(queries.changed, 7, "{queries:?}");
        assert!(queries.rounds <= 4, "{queries:?}");
        assert_eq!(queries.fallbacks, 0, "{queries:?}");
        assert_eq!(nest.buckets(), vec![Some(0); 7]);
        // The same page laid out from nothing at 300 px.
        let mut cold = Panel::nest(300.0, 1_000, true);
        assert_eq!(cold.buckets(), vec![Some(0); 7]);
        assert_matches_cold(&mut nest.context, &mut cold.context, nest.document);

        // Still below every breakpoint: rules are read, nothing changes.
        let queries = Queries::from(nest.resize(320.0));
        assert_eq!(queries.changed, 0, "{queries:?}");
        assert_eq!(queries.downstream, 0, "{queries:?}");
    }

    #[test]
    fn issue265_a_rule_that_moves_its_own_container_settles_in_its_frame() {
        // A box sized by its one child, which is wide while the box is narrow
        // and narrow while it is wide: the rule cannot be satisfied.
        let run = || {
            let document = DocumentId::new(1).unwrap();
            let mut queue = MutationQueue::new();
            queue.create(node(1), document, NodeKind::Document);
            // A row: the box in it is as wide as what it holds.
            element(
                &mut queue,
                document,
                node(1),
                node(2),
                styled(LayoutStyle {
                    direction: Some(FlexDirection::Row),
                    ..column(Some(1200.0), Some(800.0))
                }),
            );
            element(
                &mut queue,
                document,
                node(2),
                node(3),
                styled(column(None, None)),
            );
            element(
                &mut queue,
                document,
                node(3),
                node(4),
                styled(fixed(100.0, 20.0)),
            );
            queue.set_responsive(
                node(4),
                Some(Arc::new(
                    ResponsiveRule::new(ResponsiveContainer::Parent, ResponsiveAxis::Inline)
                        .below(200.0, |layout| layout.width = Some(LengthSpec::Px(300.0))),
                )),
            );
            let mut context = AppContext::new();
            context.compat_world_mut().commit(queue).unwrap();
            let mut shaper = bundled_face_shaper();
            let counters = product_frame(&mut context, document, viewport(), &mut shaper);
            let passes = context.layout_invocations();
            product_frame(&mut context, document, viewport(), &mut shaper);
            assert_eq!(
                context.layout_invocations(),
                passes,
                "the next frame is idle"
            );
            let world = context.world();
            (
                Queries::from(counters),
                world.responsive_bucket(node(4)),
                world.layout_box(node(3)),
                world.layout_box(node(4)),
            )
        };
        let (queries, bucket, container, child) = run();
        assert!(queries.fallbacks >= 1, "{queries:?}");
        assert!(queries.rounds <= 4, "{queries:?}");
        assert!(bucket.is_some());
        // The same inputs settle the same way.
        assert_eq!(run(), (queries, bucket, container, child));
    }

    #[test]
    fn issue265_a_variant_hides_optional_chrome_and_style_follows_it() {
        // Below 300 px the panel drops its optional row: the row's computed
        // style, not only its box, follows the variant.
        let mut panel = Panel::new(1_000, 320.0, true);
        let chrome = row(RULES - 1);
        let mut queue = MutationQueue::new();
        queue.set_responsive(
            chrome,
            Some(Arc::new(
                ResponsiveRule::new(ResponsiveContainer::Parent, ResponsiveAxis::Inline)
                    .below(300.0, |layout| layout.hidden = true),
            )),
        );
        panel.context.commit_mutations(queue).unwrap();
        panel.frame();
        let visible = |panel: &Panel| {
            panel
                .context
                .world()
                .computed_style(chrome)
                .unwrap()
                .visible
        };
        assert!(visible(&panel));
        assert_eq!(panel.height(RULES - 1), 24.0);

        let queries = Queries::from(panel.resize(280.0));
        assert!(queries.changed >= 1, "{queries:?}");
        assert!(!visible(&panel), "the hidden variant hides the row");
        assert_eq!(panel.height(RULES - 1), 0.0, "and omits its box");

        panel.resize(320.0);
        assert!(visible(&panel));
        assert_eq!(panel.height(RULES - 1), 24.0);
    }

    #[test]
    fn issue265_a_rule_follows_its_node_to_a_new_parent_and_can_be_dropped() {
        let mut panel = Panel::new(1_000, 520.0, true);
        let moved = row(0);
        assert_eq!(panel.height(0), 24.0, "520 px is at its breakpoint");
        // A narrow box beside the panel: the row reads it once it moves there.
        let narrow = node(PANEL - 1);
        let mut queue = MutationQueue::new();
        element(
            &mut queue,
            DocumentId::new(1).unwrap(),
            node(2),
            narrow,
            styled(column(Some(300.0), Some(100.0))),
        );
        panel.context.commit_mutations(queue).unwrap();
        panel.frame();
        let mut queue = MutationQueue::new();
        queue.insert(narrow, moved, None);
        panel.context.commit_mutations(queue).unwrap();
        panel.frame();
        assert_eq!(panel.height(0), 36.0, "300 px is below its breakpoint");
        assert_eq!(panel.context.world().responsive_bucket(moved), Some(0));

        // Without its rule the row is its own layout again.
        let mut queue = MutationQueue::new();
        queue.set_responsive(moved, None);
        panel.context.commit_mutations(queue).unwrap();
        panel.frame();
        assert_eq!(panel.height(0), 24.0);
        assert_eq!(panel.context.world().responsive_bucket(moved), None);
        assert_eq!(panel.footprint().0, RULES as usize - 1);
    }

    /// A variant made of data, as CSS writes one: the fields it changes and their
    /// values. A paint-only variant paints and lays nothing out; the node keeps
    /// the style it was authored with, and a later style write starts from that
    /// and keeps the variant over it.
    #[test]
    fn issue265_a_data_variant_paints_without_layout_and_keeps_the_authored_style() {
        let mut panel = Panel::new(1_000, 320.0, true);
        let painted = row(RULES - 1);
        let red = [1.0, 0.0, 0.0, 1.0];
        let authored = panel.context.world().node_style(painted).unwrap().clone();
        let redder = LayoutStyle {
            background: Some(red),
            ..(*authored.layout).clone()
        };
        let rule = || {
            Arc::new(
                ResponsiveRule::from_buckets(
                    ResponsiveContainer::Parent,
                    ResponsiveAxis::Width,
                    vec![300.0],
                    vec![StyleVariant::between(&authored.layout, &redder), None],
                )
                .unwrap(),
            )
        };
        let installed = rule();
        let mut queue = MutationQueue::new();
        queue.set_responsive(painted, Some(Arc::clone(&installed)));
        panel.context.commit_mutations(queue).unwrap();
        panel.frame();
        let background = |panel: &Panel| {
            panel
                .context
                .world()
                .computed_style(painted)
                .unwrap()
                .background
        };
        assert_eq!(background(&panel), None);

        let queries = Queries::from(panel.resize(280.0));
        assert!(queries.changed >= 1, "{queries:?}");
        assert_eq!(background(&panel), Some(red), "the variant paints");
        assert_eq!(
            queries.downstream, 0,
            "a paint-only variant seeds no layout"
        );
        let world = panel.context.world();
        assert_eq!(
            world.node_style(painted),
            Some(&authored),
            "authored style kept"
        );
        // The same rule sent again is the rule it has.
        let mut queue = MutationQueue::new();
        queue.set_responsive(painted, Some(rule()));
        panel.context.commit_mutations(queue).unwrap();
        assert!(Arc::ptr_eq(
            panel.context.world().responsive_rule(painted).unwrap(),
            &installed
        ));

        // A write while in the bucket starts from the authored style; the
        // variant stays written over it.
        let taller = styled(fixed(200.0, 30.0));
        let mut queue = MutationQueue::new();
        queue.set_style(painted, taller.clone());
        panel.context.commit_mutations(queue).unwrap();
        panel.frame();
        assert_eq!(panel.height(RULES - 1), 30.0);
        assert_eq!(background(&panel), Some(red));
        assert_eq!(panel.context.world().node_style(painted), Some(&taller));

        panel.resize(320.0);
        assert_eq!(background(&panel), None);
        assert_eq!(panel.height(RULES - 1), 30.0);
    }

    /// `container-type` and `container-name`: a rule reads the nearest box above
    /// it that answers its query, past boxes that do not, and keeps its authored
    /// style while none does. A box that becomes or stops being a query
    /// container is found, or lost, by the rules below it.
    #[test]
    fn issue265_a_rule_reads_the_nearest_query_container_by_name() {
        let document = DocumentId::new(1).unwrap();
        let card = node(PANEL + 500);
        let wrapper = node(PANEL + 501);
        let named = node(PANEL + 502);
        let unnamed = node(PANEL + 503);
        let elsewhere = node(PANEL + 504);
        let card_at = |width: f32| LayoutStyle {
            container_type: ContainerType::InlineSize,
            container_name: vec!["card".into()],
            ..column(Some(width), Some(400.0))
        };
        let mut queue = MutationQueue::new();
        page_with_filler(&mut queue, document, 1_000, 10);
        element(&mut queue, document, node(2), card, styled(card_at(320.0)));
        element(
            &mut queue,
            document,
            card,
            wrapper,
            styled(column(None, None)),
        );
        let tall = fixed(100.0, 24.0);
        let taller = fixed(100.0, 36.0);
        for (id, name, axis) in [
            (named, Some("card"), ResponsiveAxis::Width),
            (unnamed, None, ResponsiveAxis::Inline),
            (elsewhere, Some("sidebar"), ResponsiveAxis::Width),
        ] {
            element(&mut queue, document, wrapper, id, styled(tall.clone()));
            queue.set_responsive(
                id,
                Some(Arc::new(
                    ResponsiveRule::from_buckets(
                        ResponsiveContainer::Nearest {
                            name: name.map(String::from),
                        },
                        axis,
                        vec![300.0],
                        vec![StyleVariant::between(&tall, &taller), None],
                    )
                    .unwrap(),
                )),
            );
        }
        let mut panel = Panel::laid_out(document, queue, true, card, card_at);
        let height = |panel: &Panel, id| panel.context.world().layout_box(id).unwrap().height;
        for id in [named, unnamed, elsewhere] {
            assert_eq!(height(&panel, id), 24.0);
        }

        panel.resize(280.0);
        assert_eq!(
            height(&panel, named),
            36.0,
            "found by name, past the wrapper"
        );
        assert_eq!(height(&panel, unnamed), 36.0, "an unnamed query takes any");
        assert_eq!(
            height(&panel, elsewhere),
            24.0,
            "no box answers to its name"
        );
        assert_eq!(panel.context.world().responsive_bucket(elsewhere), None);

        // The card stops being a query container: its rules keep their
        // authored style.
        let mut queue = MutationQueue::new();
        queue.set_style(card, styled(column(Some(280.0), Some(400.0))));
        panel.context.commit_mutations(queue).unwrap();
        panel.frame();
        assert_eq!(height(&panel, named), 24.0);
        assert_eq!(panel.context.world().responsive_bucket(named), None);

        // The wrapper becomes one, by that name: it is the nearest now.
        let mut queue = MutationQueue::new();
        queue.set_style(
            wrapper,
            styled(LayoutStyle {
                container_type: ContainerType::Size,
                container_name: vec!["card".into()],
                ..column(None, None)
            }),
        );
        panel.context.commit_mutations(queue).unwrap();
        panel.frame();
        assert_eq!(height(&panel, named), 36.0, "280 px wide: below 300");
        assert_eq!(height(&panel, elsewhere), 24.0);
    }

    /// A node in a bucket is the node authored with the variant written over its
    /// style: the same computed style, the same box.
    #[test]
    fn issue265_a_node_in_a_bucket_matches_the_node_authored_that_way() {
        let mut panel = Panel::new(1_000, 280.0, true);
        let base = fixed(200.0, 24.0);
        let narrow = LayoutStyle {
            height: Some(LengthSpec::Px(40.0)),
            padding: Some(LengthSpec::Px(6.0)),
            background: Some([0.2, 0.4, 0.6, 1.0]),
            opacity: Some(0.5),
            font_size: Some(18.0),
            ..base.clone()
        };
        let following = row(RULES - 1);
        let authored = row(RULES - 2);
        let mut queue = MutationQueue::new();
        queue.set_responsive(
            following,
            Some(Arc::new(
                ResponsiveRule::from_buckets(
                    ResponsiveContainer::Parent,
                    ResponsiveAxis::Inline,
                    vec![300.0],
                    vec![StyleVariant::between(&base, &narrow), None],
                )
                .unwrap(),
            )),
        );
        queue.set_responsive(authored, None);
        queue.set_style(authored, styled(narrow));
        panel.context.commit_mutations(queue).unwrap();
        panel.frame();
        let world = panel.context.world();
        assert_eq!(
            world.computed_style(following),
            world.computed_style(authored)
        );
        let size = |id| {
            let layout = world.layout_box(id).unwrap();
            (layout.width, layout.height)
        };
        assert_eq!(size(following), size(authored));
        assert_eq!(size(following), (200.0, 40.0));
    }
}
