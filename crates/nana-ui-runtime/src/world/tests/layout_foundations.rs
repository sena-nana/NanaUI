//! Retained layout foundations: formatting contexts, containment, scroll/clip chains, sizing, and withdrawal.

#![cfg(test)]

mod formatting_context {
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

    use nana_ui_core::{DisplaySpec, FlexDirection, GridTrack, LayoutStyle, LengthSpec};

    use crate::world::reflow_oracle::{
        Builder, assert_matches_cold, bundled_face_shaper, product_frame, styled,
    };
    use crate::world::tests::replaced_media::image_layout;
    use crate::world::{DocumentId, StableNodeId, UiWorld};
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
            let children = page.world().layout_child_fragments(container).unwrap();
            assert!(!children.is_empty());
            assert!(
                children
                    .iter()
                    .all(|fragment| fragment.kind == context.child_fragment_kind()),
                "{display:?}: {children:?}"
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
}

mod context_transitions {
    //! Real retained components through context and placement transitions.

    use crate::world::reflow_oracle::{bundled_face_shaper, product_frame, styled};
    use crate::{
        AppContext, Button, CustomRenderNode, DocumentId, FormattingContextKind, LayoutViewport,
        MutationQueue, ParticipationKind, Stack, TextInput,
    };
    use nana_ui_core::{
        DisplaySpec, FlexDirection, GridTrack, LayoutStyle, LengthSpec, PositionSpec,
    };
    use std::sync::Arc;

    fn container(display: DisplaySpec, direction: FlexDirection) -> LayoutStyle {
        LayoutStyle {
            display: Some(display),
            direction: Some(direction),
            width: Some(LengthSpec::Px(600.0)),
            grid_columns: (display == DisplaySpec::Grid).then(|| vec![GridTrack::Px(40.0); 10]),
            ..Default::default()
        }
    }
    fn item(position: PositionSpec) -> LayoutStyle {
        LayoutStyle {
            display: Some(DisplaySpec::InlineBlock),
            width: Some(LengthSpec::Px(40.0)),
            height: Some(LengthSpec::Px(24.0)),
            position,
            ..Default::default()
        }
    }

    #[test]
    fn issue199_a_thousand_components_keep_state_focus_and_resource_identity() {
        let document = DocumentId::new(1).unwrap();
        let viewport = LayoutViewport::new(800.0, 600.0);
        let mut cx = AppContext::new();
        let mut shaper = bundled_face_shaper();
        let root = cx
            .create_component(
                document,
                Stack::from_layout(container(DisplaySpec::Flex, FlexDirection::Row)),
            )
            .unwrap();
        let editor = cx
            .create_detached_component(
                document,
                TextInput::new("retained draft").layout(Arc::new(item(PositionSpec::Static))),
            )
            .unwrap();
        let mut children = vec![editor.stable_id()];
        let mut resources = Vec::new();
        let mut queue = MutationQueue::new();
        for n in 1..1_000 {
            let node = if n % 100 == 0 {
                let node = cx
                    .create_detached_component(
                        document,
                        Stack::from_layout(item(PositionSpec::Static)),
                    )
                    .unwrap()
                    .stable_id();
                let descriptor =
                    CustomRenderNode::new("fixture-renderer", format!("resource-{n}"), 7);
                resources.push((node, descriptor.clone()));
                queue.set_custom_render(node, Some(descriptor));
                node
            } else {
                cx.create_detached_component(
                    document,
                    Button::new("Save").layout(Arc::new(item(PositionSpec::Static))),
                )
                .unwrap()
                .stable_id()
            };
            children.push(node);
        }
        for &child in &children {
            queue.insert(root.stable_id(), child, None);
        }
        cx.commit_mutations(queue).unwrap();
        product_frame(&mut cx, document, viewport, &mut shaper);
        cx.focus_node(document, editor.stable_id()).unwrap();
        product_frame(&mut cx, document, viewport, &mut shaper);
        let nodes = cx.world().len();
        let before = cx.read(editor, |v| v.state.clone()).unwrap();
        let mut cross_hits = 0;
        let mut cross_misses = 0;
        for (display, direction, context) in [
            (
                DisplaySpec::Grid,
                FlexDirection::Row,
                FormattingContextKind::Grid,
            ),
            (
                DisplaySpec::Layers,
                FlexDirection::Row,
                FormattingContextKind::Layers,
            ),
            (
                DisplaySpec::Block,
                FlexDirection::Row,
                FormattingContextKind::Inline,
            ),
            (
                DisplaySpec::Flex,
                FlexDirection::Row,
                FormattingContextKind::Flex,
            ),
            (
                DisplaySpec::Flex,
                FlexDirection::Column,
                FormattingContextKind::Flex,
            ),
        ] {
            let mut q = MutationQueue::new();
            q.set_style(root.stable_id(), styled(container(display, direction)));
            cx.commit_mutations(q).unwrap();
            let work = product_frame(&mut cx, document, viewport, &mut shaper);
            assert_eq!((work.entities_spawned, work.entities_despawned), (0, 0));
            assert_eq!(cx.world().len(), nodes);
            assert_eq!(cx.world().child_ids(root.stable_id()), children);
            assert_eq!(
                cx.world()
                    .layout_node(root.stable_id())
                    .unwrap()
                    .established,
                Some(context)
            );
            assert_eq!(cx.read(editor, |v| v.state.clone()).unwrap(), before);
            assert_eq!(cx.world().focused(document), Some(editor.stable_id()));
            for (id, resource) in &resources {
                let retained = cx.world().custom_render(*id).unwrap();
                assert_eq!(retained, resource);
                assert!(Arc::ptr_eq(&retained.resource, &resource.resource));
            }
            cross_hits += work.cross_context_measure_hits;
            cross_misses += work.cross_context_measure_misses;
        }
        for (position, part) in [
            (PositionSpec::Absolute, ParticipationKind::Absolute),
            (PositionSpec::Fixed, ParticipationKind::Fixed),
            (PositionSpec::Static, ParticipationKind::FlexItem),
        ] {
            let mut q = MutationQueue::new();
            for &child in &children {
                q.set_style(child, styled(item(position)));
            }
            cx.commit_mutations(q).unwrap();
            let work = product_frame(&mut cx, document, viewport, &mut shaper);
            assert_eq!((work.entities_spawned, work.entities_despawned), (0, 0));
            assert_eq!(cx.world().len(), nodes);
            for &child in &children {
                assert_eq!(
                    cx.world().layout_node(child).unwrap().participation,
                    Some(part)
                );
            }
            assert_eq!(cx.read(editor, |v| v.state.clone()).unwrap(), before);
            assert_eq!(cx.world().focused(document), Some(editor.stable_id()));
            for (id, resource) in &resources {
                assert!(Arc::ptr_eq(
                    &cx.world().custom_render(*id).unwrap().resource,
                    &resource.resource
                ));
            }
        }
        product_frame(&mut cx, document, viewport, &mut shaper);
        assert!(cx.take_system_work().is_empty());
        let idle = cx.world().counters.frame_counters;
        assert_eq!(
            (
                idle.layout_measure_nodes,
                idle.layout_placement_nodes,
                idle.layout_context_transitions,
                idle.entities_spawned,
                idle.entities_despawned
            ),
            (0, 0, 0, 0, 0)
        );
        assert_eq!(
            cross_misses, 0,
            "compatible constraints must reuse intrinsic facts"
        );
        println!(
            "1k context transitions: cross-context intrinsic hits={cross_hits}, misses={cross_misses}"
        );
    }

    /// The same container through Flex → Grid → Layers → Inline → Flex many
    /// times. What the retained cache holds settles after the first round: a
    /// context it left keeps no grid tracks or line plans, and the cache matches
    /// one that only ever laid the container out as Flex.
    #[test]
    fn issue199_repeated_context_switches_keep_the_retained_cache_flat() {
        use crate::layout_engine::RetainedLayoutFootprint;
        let document = DocumentId::new(1).unwrap();
        let viewport = LayoutViewport::new(800.0, 600.0);
        let mount = |cx: &mut AppContext| {
            let root = cx
                .create_component(
                    document,
                    Stack::from_layout(container(DisplaySpec::Flex, FlexDirection::Row)),
                )
                .unwrap();
            let mut queue = MutationQueue::new();
            for _ in 0..100 {
                let child = cx
                    .create_detached_component(
                        document,
                        Button::new("Save").layout(Arc::new(item(PositionSpec::Static))),
                    )
                    .unwrap();
                queue.insert(root.stable_id(), child.stable_id(), None);
            }
            cx.commit_mutations(queue).unwrap();
            root.stable_id()
        };
        let mut cx = AppContext::new();
        let mut shaper = bundled_face_shaper();
        let root = mount(&mut cx);
        product_frame(&mut cx, document, viewport, &mut shaper);
        let nodes = cx.world().len();
        // Intrinsic facts are the cross-context cache: kept on purpose, held to
        // their budget below instead of compared.
        let flat = |footprint: RetainedLayoutFootprint| RetainedLayoutFootprint {
            intrinsics: 0,
            intrinsic_metrics: 0,
            last_frontier: 0,
            ..footprint
        };
        let round = [
            DisplaySpec::Grid,
            DisplaySpec::Layers,
            DisplaySpec::Block,
            DisplaySpec::Flex,
        ];
        let mut settled = None;
        let mut in_grid = None;
        for _ in 0..20 {
            for display in round {
                let mut q = MutationQueue::new();
                q.set_style(root, styled(container(display, FlexDirection::Row)));
                cx.commit_mutations(q).unwrap();
                let work = product_frame(&mut cx, document, viewport, &mut shaper);
                assert_eq!(work.layout_context_transitions, 1, "{display:?}");
                if display == DisplaySpec::Grid {
                    in_grid.get_or_insert(flat(cx.retained_layout_footprint(document)));
                }
            }
            let footprint = cx.retained_layout_footprint(document);
            assert_eq!(footprint.full_snapshots, 0);
            assert!(footprint.intrinsics <= 2 * nodes, "{footprint:?}");
            assert!(footprint.intrinsic_metrics <= 4 * nodes, "{footprint:?}");
            match settled {
                None => settled = Some(flat(footprint)),
                Some(settled) => assert_eq!(flat(footprint), settled),
            }
        }
        let mut cold = AppContext::new();
        mount(&mut cold);
        product_frame(&mut cold, document, viewport, &mut shaper);
        assert_eq!(
            settled.unwrap(),
            flat(cold.retained_layout_footprint(document)),
            "back in Flex, nothing from Grid, Layers or Inline is left"
        );
        // The comparison sees context state: while in Grid the cache held more.
        let (grid, flex) = (in_grid.unwrap(), settled.unwrap());
        assert!(
            grid.plan_entries > flex.plan_entries,
            "{grid:?} vs {flex:?}"
        );
    }

    /// A modal frame establishes the Overlay context through the same
    /// dispatch and participation contract as Flex and Grid: its slots are
    /// `OverlayItem`s,
    /// placed by the slot contract, and an incremental change to a slot matches a
    /// cold layout of the same dialog.
    #[test]
    fn issue199_a_modal_frame_places_its_slots_as_overlay_items() {
        use crate::world::reflow_oracle::assert_matches_cold;
        use crate::{Dialog, ModalSlots, OverlayHost};
        let document = DocumentId::new(1).unwrap();
        let viewport = LayoutViewport::new(800.0, 600.0);
        let body_layout = |height: f32| LayoutStyle {
            width: Some(LengthSpec::Px(240.0)),
            height: Some(LengthSpec::Px(height)),
            ..Default::default()
        };
        let build = |cx: &mut AppContext| {
            let host = cx.create_component(document, OverlayHost::new()).unwrap();
            let dialog = cx
                .create_component(document, Dialog::new("Settings"))
                .unwrap();
            let body = cx
                .create_detached_component(document, Stack::from_layout(body_layout(80.0)))
                .unwrap();
            let action = cx
                .create_detached_component(document, Button::new("Save"))
                .unwrap();
            cx.append_child(host, dialog).unwrap();
            cx.set_modal_slots(
                dialog,
                ModalSlots {
                    body: Some(body.stable_id()),
                    actions: vec![action.stable_id()],
                    ..Default::default()
                },
            )
            .unwrap();
            cx.activate_overlay(host, dialog).unwrap();
            (dialog.stable_id(), body.stable_id(), action.stable_id())
        };
        let mut shaper = bundled_face_shaper();
        let mut cx = AppContext::new();
        let (dialog, body, action) = build(&mut cx);
        product_frame(&mut cx, document, viewport, &mut shaper);
        let world = cx.world();
        assert_eq!(
            world.layout_node(dialog).unwrap().established,
            Some(FormattingContextKind::Overlay)
        );
        for slot in [body, action] {
            assert_eq!(
                world.layout_node(slot).unwrap().participation,
                Some(ParticipationKind::OverlayItem)
            );
        }
        assert!((world.layout_box(body).unwrap().height - 80.0).abs() < 0.01);

        // A taller body grows the surface around it.
        let mut q = MutationQueue::new();
        q.set_style(body, styled(body_layout(120.0)));
        cx.commit_mutations(q).unwrap();
        let work = product_frame(&mut cx, document, viewport, &mut shaper);
        // Relaying the same dialog out is not a context change.
        assert_eq!(work.layout_context_transitions, 0);
        // The slot contract measures through the surface's float chrome.
        assert!((cx.world().layout_box(body).unwrap().height - 120.0).abs() < 0.01);

        let mut cold = AppContext::new();
        let (_, cold_body, _) = build(&mut cold);
        let mut q = MutationQueue::new();
        q.set_style(cold_body, styled(body_layout(120.0)));
        cold.commit_mutations(q).unwrap();
        product_frame(&mut cold, document, viewport, &mut shaper);
        assert_matches_cold(&mut cx, &mut cold, document);
    }
}

mod containing_block {
    //! Issue #200: placement shares one coordinate authority.
    //!
    //! An absolute box hangs from its containing block as CSS defines it: the
    //! padding box of its nearest positioned ancestor, static ones in between
    //! skipped, or the initial containing block -- the viewport at the document
    //! origin -- when it has none. A retained pass follows that block when it
    //! moves or resizes, and leaves the box where it is when only a static box
    //! between them moves, matching a cold layout at every step.
    //!
    //! A pointer menu and an anchored action menu state an anchor
    //! ([`nana_ui_core::AnchorSpec`]). Layout places them, at the size they
    //! measure and against the real viewport, through the same
    //! [`crate::resolve_anchored_surface`] that triggered menus, hover cards,
    //! popovers and the select drop-down use. They flip to the side that fits,
    //! stay in the window (and in the room an owner gives), follow a viewport
    //! change with no reprojection, and hit testing and accessibility read the
    //! box layout gave them.
    //!
    //! A fixed box hangs from its own document's viewport, past positioned and
    //! scrolled ancestors, and so does an absolute box with no positioned
    //! ancestor. Two documents of one context keep their own viewports: a
    //! resize of one runs no layout in the other, and hit testing and
    //! accessibility read the boxes each viewport gave.
    //!
    //! An open Select, Dropdown or SearchDropdown menu reads as a combobox's
    //! list box: one option per row the menu shows, at the bounds its menu
    //! geometry paints and hits, following the menu when the window flips it.

    use nana_ui_core::{
        AnchoredMenuPlacement, FlexDirection, LayoutStyle, LengthSpec, PositionSpec, WorkCounters,
    };

    use crate::world::reflow_oracle::{
        Builder, assert_matches_cold, bundled_face_shaper, product_frame, resize_frame, styled,
    };
    use crate::world::{DocumentId, StableNodeId};
    use crate::{
        AccessibilityRole, AnchoredActionMenu, AppContext, ComponentView, ContextMenu,
        ContextMenuItem, Entity, LayoutBox, LayoutViewport, MutationQueue, NanaTextEngineShaper,
        OverlayHost, resolve_anchored_origin,
    };

    /// A window whose overlay layer hosts one open menu, as an application
    /// mounts it. Motion is reduced so the surface shows at its laid-out size.
    struct Window {
        context: AppContext,
        document: DocumentId,
        host: Entity<OverlayHost>,
        shaper: NanaTextEngineShaper,
        viewport: LayoutViewport,
    }

    impl Window {
        fn new(width: f32, height: f32) -> Self {
            let mut context = AppContext::new();
            context.set_reduced_motion(true);
            let document = DocumentId::new(1).unwrap();
            let host = context
                .create_component(document, OverlayHost::new())
                .unwrap();
            Self {
                context,
                document,
                host,
                shaper: bundled_face_shaper(),
                viewport: LayoutViewport::new(width, height),
            }
        }

        fn open<C: ComponentView>(&mut self, surface: C) -> StableNodeId {
            let surface = self
                .context
                .create_component(self.document, surface)
                .unwrap();
            self.context.append_child(self.host, surface).unwrap();
            self.context.activate_overlay(self.host, surface).unwrap();
            surface.stable_id()
        }

        fn frame(&mut self) {
            product_frame(
                &mut self.context,
                self.document,
                self.viewport,
                &mut self.shaper,
            );
        }

        fn resize(&mut self, width: f32, height: f32) {
            self.viewport = LayoutViewport::new(width, height);
            resize_frame(
                &mut self.context,
                self.document,
                self.viewport,
                &mut self.shaper,
            );
        }

        fn surface(&self, id: StableNodeId) -> LayoutBox {
            self.context.world().layout_box(id).unwrap()
        }

        fn window_box(&self) -> LayoutBox {
            LayoutBox {
                x: 0.0,
                y: 0.0,
                width: self.viewport.width,
                height: self.viewport.height,
            }
        }
    }

    fn items(count: usize) -> Vec<ContextMenuItem> {
        (0..count)
            .map(|index| ContextMenuItem::new(format!("item{index}"), format!("Item {index}")))
            .collect()
    }

    fn inside(surface: LayoutBox, room: LayoutBox) -> bool {
        surface.x >= room.x - 0.01
            && surface.y >= room.y - 0.01
            && surface.x + surface.width <= room.x + room.width + 0.01
            && surface.y + surface.height <= room.y + room.height + 0.01
    }

    #[test]
    fn issue200_a_pointer_menu_is_placed_by_layout_and_follows_the_viewport() {
        let mut window = Window::new(800.0, 600.0);
        let (x, y) = (760.0, 560.0);
        let menu = window.open(ContextMenu::new(x, y).items(items(6)).open(true));
        window.frame();
        let surface = window.surface(menu);
        // No room below or after the pointer: it opens above it and moves left
        // into the window, where the shared resolver puts a box of its size.
        assert!(surface.height > 0.0);
        assert!((surface.y + surface.height - y).abs() < 0.01, "{surface:?}");
        assert!(inside(surface, window.window_box()), "{surface:?}");
        assert_eq!(
            (surface.x, surface.y),
            resolve_anchored_origin(
                x,
                y,
                surface.width,
                surface.height,
                window.window_box(),
                AnchoredMenuPlacement::BottomStart,
            )
        );
        // Hit testing and accessibility read the box layout gave it.
        window.context.rebuild_hit_test(window.document);
        let world = window.context.world();
        let center = (
            surface.x + surface.width / 2.0,
            surface.y + surface.height / 2.0,
        );
        let mut hit = world.hit_test(window.document, center.0, center.1);
        while hit.is_some_and(|node| node != menu) {
            hit = hit.and_then(|node| world.parent_id(node));
        }
        assert_eq!(hit, Some(menu), "the pointer over the menu lands in it");
        let spoken = world
            .project_accessibility_in_viewport(window.document)
            .into_iter()
            .find(|node| node.id == menu)
            .unwrap()
            .bounds;
        assert_eq!(
            (spoken.x, spoken.y, spoken.width, spoken.height),
            (surface.x, surface.y, surface.width, surface.height)
        );

        // A taller window has room below the pointer: layout moves the menu
        // there on the resize alone, with no reprojection of the component.
        let authored = |window: &Window| {
            window
                .context
                .world()
                .node_style(menu)
                .unwrap()
                .layout
                .clone()
        };
        let before = authored(&window);
        window.resize(1200.0, 900.0);
        let moved = window.surface(menu);
        assert_eq!((moved.x, moved.y), (x, y), "{moved:?}");
        assert!(
            std::sync::Arc::ptr_eq(&before, &authored(&window)),
            "the menu's authored style is the one it had before the resize"
        );
    }

    #[test]
    fn issue200_a_menu_stays_in_the_room_its_owner_gives() {
        let mut window = Window::new(800.0, 600.0);
        let room = LayoutBox {
            x: 100.0,
            y: 100.0,
            width: 300.0,
            height: 240.0,
        };
        let mut menu = ContextMenu::new(380.0, 300.0).items(items(5)).open(true);
        menu.place_in(room);
        let menu = window.open(menu);
        window.frame();
        let surface = window.surface(menu);
        assert!(inside(surface, room), "{surface:?} outside {room:?}");
        assert_eq!(
            (surface.x, surface.y),
            resolve_anchored_origin(
                380.0,
                300.0,
                surface.width,
                surface.height,
                room,
                AnchoredMenuPlacement::BottomStart,
            )
        );
    }

    #[test]
    fn issue200_an_anchored_action_menu_hangs_on_its_side_and_flips() {
        let hang = |x: f32, y: f32| {
            let mut window = Window::new(800.0, 600.0);
            let menu = window.open(
                AnchoredActionMenu::new(x, y)
                    .placement(AnchoredMenuPlacement::TopEnd)
                    .menu_size(160.0, 120.0)
                    .open(true),
            );
            window.frame();
            window.surface(menu)
        };
        // Room above and before the point: the menu ends on it.
        let surface = hang(300.0, 300.0);
        assert_eq!(
            (
                surface.x + surface.width,
                surface.y + surface.height,
                surface.width,
                surface.height
            ),
            (300.0, 300.0, 160.0, 120.0)
        );
        // Top-end from a point near the top-left corner: no room above, so it
        // flips below, and its end alignment would leave the window, so it moves
        // right into it.
        let surface = hang(40.0, 20.0);
        assert_eq!((surface.x, surface.y), (0.0, 20.0), "{surface:?}");
    }

    /// A tree given as `(parent, style)` per node -- `None` for the page, an
    /// index into the list otherwise -- laid out through the product frame. Each
    /// step restyles the retained tree and compares it with one built cold.
    struct Tree {
        spec: Vec<(Option<usize>, LayoutStyle)>,
        context: AppContext,
        ids: Vec<StableNodeId>,
        shaper: NanaTextEngineShaper,
        viewport: LayoutViewport,
    }

    impl Tree {
        fn new(spec: Vec<(Option<usize>, LayoutStyle)>) -> Self {
            let viewport = LayoutViewport::new(800.0, 600.0);
            let (context, ids) = Self::build(&spec, viewport);
            Self {
                spec,
                context,
                ids,
                shaper: bundled_face_shaper(),
                viewport,
            }
        }

        fn document() -> DocumentId {
            DocumentId::new(1).unwrap()
        }

        fn build(
            spec: &[(Option<usize>, LayoutStyle)],
            viewport: LayoutViewport,
        ) -> (AppContext, Vec<StableNodeId>) {
            let (mut b, page) = Builder::new(Self::document(), 1);
            let page = b.element(
                page,
                LayoutStyle {
                    direction: Some(FlexDirection::Column),
                    ..LayoutStyle::default()
                },
            );
            let mut ids: Vec<StableNodeId> = Vec::with_capacity(spec.len());
            for (parent, style) in spec {
                let parent = parent.map_or(page, |index| ids[index]);
                ids.push(b.element(parent, style.clone()));
            }
            let mut context = AppContext::new();
            context.commit_mutations(b.queue).unwrap();
            product_frame(
                &mut context,
                Self::document(),
                viewport,
                &mut bundled_face_shaper(),
            );
            (context, ids)
        }

        fn rect(&self, index: usize) -> (f32, f32, f32, f32) {
            let b = self.context.world().layout_box(self.ids[index]).unwrap();
            (b.x, b.y, b.width, b.height)
        }

        /// Restyles node `index` in the retained tree, lays it out, and checks
        /// it against the same tree laid out cold.
        fn restyle(&mut self, index: usize, style: LayoutStyle) {
            self.spec[index].1 = style.clone();
            let mut queue = MutationQueue::new();
            queue.set_style(self.ids[index], styled(style));
            self.context.commit_mutations(queue).unwrap();
            product_frame(
                &mut self.context,
                Self::document(),
                self.viewport,
                &mut self.shaper,
            );
            self.matches_cold();
        }

        fn resize(&mut self, width: f32, height: f32) {
            self.viewport = LayoutViewport::new(width, height);
            resize_frame(
                &mut self.context,
                Self::document(),
                self.viewport,
                &mut self.shaper,
            );
            self.matches_cold();
        }

        fn matches_cold(&mut self) {
            let (mut cold, _) = Self::build(&self.spec, self.viewport);
            assert_matches_cold(&mut self.context, &mut cold, Self::document());
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

    /// A card that is the containing block, with padding and a border.
    fn card(position: PositionSpec, width: f32) -> LayoutStyle {
        LayoutStyle {
            position,
            padding_top: Some(LengthSpec::Px(12.0)),
            padding_right: Some(LengthSpec::Px(12.0)),
            padding_bottom: Some(LengthSpec::Px(12.0)),
            padding_left: Some(LengthSpec::Px(12.0)),
            border_width: Some(2.0),
            direction: Some(FlexDirection::Column),
            ..sized(width, 200.0)
        }
    }

    /// A static box with padding of its own between the card and the badge.
    fn wrapper(position: PositionSpec) -> LayoutStyle {
        LayoutStyle {
            position,
            padding_top: Some(LengthSpec::Px(10.0)),
            padding_left: Some(LengthSpec::Px(10.0)),
            ..sized(200.0, 100.0)
        }
    }

    /// Half as wide as its containing block, in its top-right corner.
    fn badge() -> LayoutStyle {
        LayoutStyle {
            position: PositionSpec::Absolute,
            offset_top: Some(LengthSpec::Px(0.0)),
            offset_right: Some(LengthSpec::Px(0.0)),
            width: Some(LengthSpec::Percent(50.0)),
            height: Some(LengthSpec::Px(10.0)),
            ..LayoutStyle::default()
        }
    }

    const CARD: usize = 0;
    const SPACER: usize = 1;
    const WRAPPER: usize = 2;
    const BADGE: usize = 3;

    /// The card holds a spacer and then the static box; the badge and a row in
    /// flow are in the static box, so a pass where only the static box moves
    /// replays its plan rather than placing it again.
    fn card_tree() -> Tree {
        Tree::new(vec![
            (None, card(PositionSpec::Relative, 300.0)),
            (Some(CARD), sized(200.0, 30.0)),
            (Some(CARD), wrapper(PositionSpec::Static)),
            (Some(WRAPPER), badge()),
            (Some(WRAPPER), sized(80.0, 20.0)),
        ])
    }

    /// The badge sits in the top-right corner of the card's padding box, half
    /// its width, though a static box with padding of its own stands between.
    fn assert_hangs_from_card(tree: &Tree) {
        let (cx, cy, cw, _) = tree.rect(CARD);
        let (bx, by, bw, bh) = tree.rect(BADGE);
        let inner = cw - 4.0;
        assert_eq!(
            (bx, by, bw, bh),
            (cx + 2.0 + inner / 2.0, cy + 2.0, inner / 2.0, 10.0),
            "the badge hangs from the card's padding box"
        );
    }

    #[test]
    fn issue200_an_absolute_box_hangs_from_its_nearest_positioned_ancestor() {
        let mut tree = card_tree();
        tree.matches_cold();
        assert_hangs_from_card(&tree);
        // The card grows; the static box between them does not move or resize,
        // so no replay of its plan reaches the badge -- the badge follows anyway.
        tree.restyle(CARD, card(PositionSpec::Relative, 360.0));
        assert_eq!(tree.rect(WRAPPER).2, 200.0);
        assert_hangs_from_card(&tree);
        // The static box moves down; the card does not, and neither does the
        // badge.
        let before = tree.rect(BADGE);
        tree.restyle(SPACER, sized(200.0, 50.0));
        assert_eq!(
            tree.rect(BADGE),
            before,
            "only its containing block moves it"
        );
        // The static box becomes positioned: it is the nearest one now.
        tree.restyle(WRAPPER, wrapper(PositionSpec::Relative));
        let (wx, wy, ww, _) = tree.rect(WRAPPER);
        assert_eq!(tree.rect(BADGE), (wx + ww / 2.0, wy, ww / 2.0, 10.0));
        // Neither is positioned: the initial containing block, the viewport.
        tree.restyle(WRAPPER, wrapper(PositionSpec::Static));
        tree.restyle(CARD, card(PositionSpec::Static, 360.0));
        assert_eq!(tree.rect(BADGE), (400.0, 0.0, 400.0, 10.0));
        // And back: the card is its containing block again.
        tree.restyle(CARD, card(PositionSpec::Relative, 360.0));
        assert_hangs_from_card(&tree);
    }

    #[test]
    fn issue200_a_box_hanging_past_its_parent_moves_that_parents_overflow_on_the_page() {
        // The badge sits in a static wrapper below a spacer and hangs from the
        // card's top right corner, so its box is measured from the card: in
        // those numbers it lies inside the wrapper's own box, while on the page
        // it is above it. The card narrows and the badge follows it, the
        // wrapper's box and its result unchanged (a parent result holds no
        // child boxes); the wrapper's overflow on the page follows the badge.
        let pinned = LayoutStyle {
            position: PositionSpec::Absolute,
            offset_top: Some(LengthSpec::Px(0.0)),
            offset_right: Some(LengthSpec::Px(0.0)),
            ..sized(20.0, 10.0)
        };
        let mut tree = Tree::new(vec![
            (None, card(PositionSpec::Relative, 340.0)),
            (Some(0), sized(200.0, 50.0)),
            (Some(0), sized(300.0, 100.0)),
            (Some(2), pinned),
        ]);
        tree.matches_cold();
        for width in [320.0, 300.0] {
            tree.restyle(0, card(PositionSpec::Relative, width));
            let (x, y, width, height) = tree.rect(3);
            let wrapper = tree.rect(2);
            assert!(y + height <= wrapper.1, "the badge is above its parent");
            let overflow = tree
                .context
                .world()
                .layout_result(tree.ids[2])
                .unwrap()
                .overflow;
            assert!(
                overflow.x <= x
                    && overflow.y <= y
                    && overflow.x + overflow.width >= x + width
                    && overflow.y + overflow.height >= y + height,
                "{overflow:?} holds the badge at {x}, {y}"
            );
        }
    }

    #[test]
    fn issue200_a_sticky_box_is_a_containing_block() {
        let mut tree = Tree::new(vec![
            (
                None,
                LayoutStyle {
                    offset_top: Some(LengthSpec::Px(0.0)),
                    ..card(PositionSpec::Sticky, 300.0)
                },
            ),
            (Some(CARD), sized(200.0, 30.0)),
            (Some(CARD), wrapper(PositionSpec::Static)),
            (Some(WRAPPER), badge()),
        ]);
        tree.matches_cold();
        assert_hangs_from_card(&tree);
    }

    #[test]
    fn issue200_with_no_positioned_ancestor_the_viewport_is_the_containing_block() {
        let corner = LayoutStyle {
            position: PositionSpec::Absolute,
            offset_right: Some(LengthSpec::Px(0.0)),
            offset_bottom: Some(LengthSpec::Px(0.0)),
            ..sized(30.0, 20.0)
        };
        let mut tree = Tree::new(vec![
            (None, sized(300.0, 200.0)),
            (Some(0), wrapper(PositionSpec::Static)),
            (Some(1), corner),
        ]);
        tree.matches_cold();
        assert_eq!(tree.rect(2), (770.0, 580.0, 30.0, 20.0));
        // The viewport resizes; nothing between the box and the document reads
        // it, and the box still follows.
        tree.resize(1000.0, 700.0);
        assert_eq!(tree.rect(2), (970.0, 680.0, 30.0, 20.0));
    }

    #[test]
    fn issue200_an_axis_with_auto_insets_keeps_its_static_position() {
        // Right-aligned against the card, with no vertical inset: it stays at
        // the top of the static box's content, where it would start in flow.
        let pinned = LayoutStyle {
            position: PositionSpec::Absolute,
            offset_right: Some(LengthSpec::Px(0.0)),
            ..sized(20.0, 10.0)
        };
        let mut tree = Tree::new(vec![
            (None, card(PositionSpec::Relative, 300.0)),
            (Some(CARD), sized(200.0, 30.0)),
            (Some(CARD), wrapper(PositionSpec::Static)),
            (Some(WRAPPER), pinned),
            (Some(WRAPPER), sized(80.0, 20.0)),
        ]);
        tree.matches_cold();
        let at = |tree: &Tree| {
            let (cx, _, cw, _) = tree.rect(CARD);
            let (_, wy, _, _) = tree.rect(WRAPPER);
            (cx + cw - 2.0 - 20.0, wy + 10.0)
        };
        let (x, y, ..) = tree.rect(BADGE);
        assert_eq!((x, y), at(&tree));
        // The static box moves: the box keeps its static position in it, and
        // its containing block's right edge.
        tree.restyle(SPACER, sized(200.0, 50.0));
        let (x, y, ..) = tree.rect(BADGE);
        assert_eq!((x, y), at(&tree));
    }

    /// A page with a 500 px spacer and then an open Select of eight options, laid
    /// out in a window of `height`, its hit index rebuilt.
    struct SelectPage {
        context: AppContext,
        document: DocumentId,
        select: Entity<crate::Select>,
        shaper: NanaTextEngineShaper,
    }

    impl SelectPage {
        fn new(height: f32) -> Self {
            Self::with_options(
                height,
                (0..8).map(|n| crate::SelectOption::new(format!("v{n}"), format!("Option {n}"))),
            )
        }

        /// The page with an open Select of `options`, holding `v0`.
        fn with_options(
            height: f32,
            options: impl IntoIterator<Item = crate::SelectOption>,
        ) -> Self {
            use crate::{Select, Stack};
            let document = DocumentId::new(1).unwrap();
            let mut context = AppContext::new();
            context.set_reduced_motion(true);
            let root = context
                .create_component(
                    document,
                    Stack::column(0.0).with_layout(|layout| {
                        layout.width = Some(LengthSpec::Px(400.0));
                    }),
                )
                .unwrap();
            let spacer = context
                .create_detached_component(
                    document,
                    Stack::column(0.0).with_layout(|layout| {
                        layout.width = Some(LengthSpec::Fill);
                        layout.height = Some(LengthSpec::Px(500.0));
                    }),
                )
                .unwrap();
            context.append_child(root, spacer).unwrap();
            let select = context
                .create_detached_component(document, Select::new(Some("v0")).options(options))
                .unwrap();
            context.append_child(root, select).unwrap();
            let mut page = Self {
                context,
                document,
                select,
                shaper: bundled_face_shaper(),
            };
            page.frame(height, false);
            assert!(page.context.toggle_select(select).unwrap());
            page.frame(height, false);
            page
        }

        fn frame(&mut self, height: f32, resized: bool) {
            let viewport = LayoutViewport::new(400.0, height);
            if resized {
                resize_frame(&mut self.context, self.document, viewport, &mut self.shaper);
            } else {
                product_frame(&mut self.context, self.document, viewport, &mut self.shaper);
            }
            self.context.rebuild_hit_test(self.document);
        }

        fn id(&self) -> StableNodeId {
            self.select.stable_id()
        }

        fn field(&self) -> LayoutBox {
            self.context.world().layout_box(self.id()).unwrap()
        }

        fn menu(&self) -> crate::SelectMenuGeometry {
            self.context.open_select_menu(self.id()).unwrap()
        }

        /// The row the menu paints at `(x, y)`, the option the pointer picks
        /// there, and whether hit testing gives the point to the field.
        fn at(&self, x: f32, y: f32) -> (Option<usize>, Option<usize>, bool) {
            // The field reads as its laid-out box, wherever its menu hangs.
            let spoken = self
                .context
                .world()
                .project_accessibility_in_viewport(self.document)
                .into_iter()
                .find(|node| node.id == self.id())
                .unwrap()
                .bounds;
            assert_eq!(spoken, self.field());
            let menu = self.menu();
            let painted = menu.viewport.contains(x, y).then(|| {
                menu.options
                    .iter()
                    .position(|option| option.bounds.contains(x, y))
            });
            let picked = crate::select::select_option_at(&menu, x, y);
            let hit = self.context.world().hit_test(self.document, x, y) == Some(self.id());
            (painted.flatten(), picked, hit)
        }
    }

    fn row_centre(menu: &crate::SelectMenuGeometry, row: usize) -> (f32, f32) {
        let bounds = menu.options[row].bounds;
        (
            bounds.x + bounds.width / 2.0,
            bounds.y + bounds.height / 2.0,
        )
    }

    #[test]
    fn issue200_a_select_menu_follows_the_viewport_and_hits_where_it_paints() {
        // A tall window: the menu hangs below the field.
        let mut page = SelectPage::new(1000.0);
        let field = page.field();
        let below = page.menu();
        assert_eq!(below.surface.y, field.y + field.height, "{below:?}");
        let low_row = row_centre(&below, 3);
        assert_eq!(page.at(low_row.0, low_row.1), (Some(3), Some(3), true));

        // The window shrinks under the open menu. Nothing about the field
        // changes; the menu flips above it against the window it is in now.
        page.frame(600.0, true);
        assert_eq!(page.field(), field);
        let above = page.menu();
        assert_eq!(above.surface.y + above.surface.height, field.y, "{above:?}");
        assert!(above.surface.y >= 0.0);
        // The row painted under a point is the one the pointer picks there and
        // the one hit testing gives to the field; where the menu was, it is not.
        let high_row = row_centre(&above, 3);
        assert_eq!(page.at(high_row.0, high_row.1), (Some(3), Some(3), true));
        assert_eq!(page.at(low_row.0, low_row.1), (None, None, false));
        // The same window laid out cold places the same menu.
        assert_eq!(SelectPage::new(600.0).menu(), above);

        // Back: it hangs below again, and the pointer follows it.
        page.frame(1000.0, true);
        assert_eq!(page.menu(), below);
        assert_eq!(page.at(low_row.0, low_row.1), (Some(3), Some(3), true));
        assert_eq!(page.at(high_row.0, high_row.1), (None, None, false));
        // A press there chooses the row painted under it.
        assert!(
            page.context
                .activate_select_at(page.select, low_row.0, low_row.1)
                .unwrap()
        );
        assert_eq!(
            page.context
                .read(page.select, |select| select.value.clone())
                .unwrap()
                .as_deref(),
            Some("v3")
        );
    }

    // An open Select, Dropdown or SearchDropdown menu reads as a WAI-ARIA
    // combobox's list box: a child of the field holding one option per row the
    // menu shows, each known by its value, at the bounds the menu paints and hits
    // it. The field says it is expanded and which option the keyboard is on;
    // closing the menu takes the list box away, and choosing an option through
    // accessibility is a press on its row.

    /// Every accessibility node of `document`, by id, its bounds in the
    /// viewport.
    fn spoken(
        context: &AppContext,
        document: DocumentId,
    ) -> std::collections::HashMap<StableNodeId, crate::AccessibilityNode> {
        context
            .world()
            .project_accessibility_in_viewport(document)
            .into_iter()
            .map(|node| (node.id, node))
            .collect()
    }

    /// [`spoken`] as a host holds it: bounds relative to the parent's.
    fn held(
        context: &AppContext,
        document: DocumentId,
    ) -> std::collections::HashMap<StableNodeId, crate::AccessibilityNode> {
        context
            .world()
            .project_accessibility(document)
            .into_iter()
            .map(|node| (node.id, node))
            .collect()
    }

    /// `field`'s node, the list box its open menu shows and the options in it,
    /// in row order. The field is an expanded combobox that owns the list box.
    fn listbox(
        nodes: &std::collections::HashMap<StableNodeId, crate::AccessibilityNode>,
        field: StableNodeId,
    ) -> (
        crate::AccessibilityNode,
        crate::AccessibilityNode,
        Vec<crate::AccessibilityNode>,
    ) {
        let combo = nodes[&field].clone();
        assert_eq!(combo.role, AccessibilityRole::ComboBox);
        assert_eq!(combo.expanded, Some(true), "{combo:?}");
        assert_eq!(combo.selected, None, "a combobox is not itself chosen");
        let id = crate::virtual_listbox_id(field).unwrap();
        assert_eq!(combo.children, [id]);
        let listbox = nodes[&id].clone();
        assert_eq!(listbox.role, AccessibilityRole::ListBox);
        assert_eq!(listbox.parent, Some(field));
        let options = listbox
            .children
            .iter()
            .map(|option| nodes[option].clone())
            .collect::<Vec<_>>();
        for option in &options {
            assert_eq!(option.role, AccessibilityRole::ListBoxOption);
            assert_eq!(option.parent, Some(id));
            assert_eq!(crate::decode_virtual_menu_item(option.id).unwrap().0, field);
        }
        (combo, listbox, options)
    }

    fn centre(bounds: LayoutBox) -> (f32, f32) {
        (
            bounds.x + bounds.width / 2.0,
            bounds.y + bounds.height / 2.0,
        )
    }

    /// The menu row an option node stands for now.
    fn row_of(context: &AppContext, field: StableNodeId, option: StableNodeId) -> usize {
        let (owner, slot) = crate::decode_virtual_menu_item(option).unwrap();
        assert_eq!(owner, field);
        context.world().virtual_menu_row_index(field, slot).unwrap()
    }

    /// Each option reads at the row the menu paints under it, and the pointer
    /// there picks that row, unless it is disabled, and hits the field.
    fn assert_reads_where_it_paints(page: &SelectPage, options: &[crate::AccessibilityNode]) {
        let menu = page.menu();
        for option in options {
            let row = row_of(&page.context, page.id(), option.id);
            let painted =
                crate::world::intersect_layout_boxes(menu.options[row].bounds, menu.viewport);
            assert_eq!(Some(option.bounds), painted, "row {row}");
            let (x, y) = centre(option.bounds);
            assert_eq!(
                page.at(x, y),
                (Some(row), (!option.disabled).then_some(row), true),
                "row {row}"
            );
        }
    }

    #[test]
    fn issue200_an_open_select_menu_reads_as_a_listbox_of_its_rows() {
        let page = SelectPage::with_options(
            1000.0,
            (0..8).map(|n| {
                let option = crate::SelectOption::new(format!("v{n}"), format!("Option {n}"));
                match n {
                    1 => option.hint("first"),
                    2 => option.disabled(true),
                    _ => option,
                }
            }),
        );
        let nodes = spoken(&page.context, page.document);
        let (field, listbox, options) = listbox(&nodes, page.id());
        let menu = page.menu();
        assert_eq!(listbox.bounds, menu.surface);
        assert_eq!(listbox.set_size, Some(8));
        assert!(!listbox.multiselectable);
        assert_eq!(options.len(), 8);
        for (n, option) in options.iter().enumerate() {
            assert_eq!(row_of(&page.context, page.id(), option.id), n);
            assert_eq!(
                option.label.as_deref(),
                Some(format!("Option {n}").as_str())
            );
            assert_eq!(option.description.as_deref(), (n == 1).then_some("first"));
            assert_eq!(option.selected, Some(n == 0), "v0 is chosen");
            assert_eq!(option.disabled, n == 2);
            assert_eq!(option.position_in_set, Some(n as u32 + 1));
            assert_eq!(option.set_size, Some(8));
            assert!(!option.focused, "the focus stays on the field");
        }
        // The highlight opens on the chosen option: the field's active
        // descendant.
        assert_eq!(field.active_descendant, Some(options[0].id));
        assert_reads_where_it_paints(&page, &options);
    }

    #[test]
    fn issue200_a_select_listbox_follows_its_menu_and_its_highlight() {
        let mut page = SelectPage::new(1000.0);
        // What a host holds: the full tree once, then the deltas each frame
        // drains.
        let mut held_nodes = held(&page.context, page.document);
        let (_, _, below) = listbox(&spoken(&page.context, page.document), page.id());
        let mut follow = |page: &mut SelectPage, height: f32, resized: bool| {
            let work = crate::world::reflow_oracle::accessible_frame(
                &mut page.context,
                page.document,
                LayoutViewport::new(400.0, height),
                &mut page.shaper,
                resized,
            );
            page.context.rebuild_hit_test(page.document);
            let delta = page.context.world().project_accessibility_delta(&work);
            for id in delta.removed {
                held_nodes.remove(&id);
            }
            held_nodes.extend(delta.updated.into_iter().map(|node| (node.id, node)));
            let fresh = held(&page.context, page.document);
            let (field, list, options) = listbox(&fresh, page.id());
            for node in std::iter::once(&field)
                .chain(std::iter::once(&list))
                .chain(&options)
            {
                assert_eq!(held_nodes.get(&node.id), Some(node), "the delta carries it");
            }
            let (field, _, options) = listbox(&spoken(&page.context, page.document), page.id());
            (field, options)
        };

        // The window shrinks: the menu flips above the field, and its options
        // read there.
        let (_, above) = follow(&mut page, 600.0, true);
        assert_eq!(
            above.iter().map(|option| option.id).collect::<Vec<_>>(),
            below.iter().map(|option| option.id).collect::<Vec<_>>()
        );
        let field = page.field();
        assert!(above[7].bounds.y + above[7].bounds.height <= field.y);
        assert!(below[0].bounds.y >= field.y + field.height);
        assert_reads_where_it_paints(&page, &above);

        // The keyboard moves the highlight: only paint changes, and the field's
        // active descendant follows it.
        page.context
            .update_component(page.select, |select, _| select.highlight_delta(1))
            .unwrap();
        let (field, options) = follow(&mut page, 600.0, false);
        assert_eq!(field.active_descendant, Some(options[1].id));

        // Back: below again.
        let (_, options) = follow(&mut page, 1000.0, true);
        assert_eq!(
            options
                .iter()
                .map(|option| option.bounds)
                .collect::<Vec<_>>(),
            below.iter().map(|option| option.bounds).collect::<Vec<_>>()
        );
        assert_reads_where_it_paints(&page, &options);
    }

    #[test]
    fn issue200_a_select_listbox_reads_only_the_rows_its_menu_shows() {
        // Forty options under a 500 px spacer in a 640 px window: the menu opens
        // above the field, capped to the room there, and scrolls its rows.
        let mut page = SelectPage::with_options(
            640.0,
            (0..40).map(|n| crate::SelectOption::new(format!("v{n}"), format!("Option {n}"))),
        );
        let check = |page: &SelectPage| {
            let menu = page.menu();
            assert!(menu.max_scroll > 0.0, "{menu:?}");
            let (_, listbox, options) = listbox(&spoken(&page.context, page.document), page.id());
            assert_eq!(listbox.set_size, Some(40));
            let shown = menu
                .options
                .iter()
                .enumerate()
                .filter(|(_, row)| {
                    crate::world::intersect_layout_boxes(row.bounds, menu.viewport).is_some()
                })
                .map(|(row, _)| row)
                .collect::<Vec<_>>();
            assert!(shown.len() < 40, "{shown:?}");
            assert_eq!(
                options
                    .iter()
                    .map(|option| row_of(&page.context, page.id(), option.id))
                    .collect::<Vec<_>>(),
                shown
            );
            for option in &options {
                let row = row_of(&page.context, page.id(), option.id);
                assert_eq!(option.position_in_set, Some(row as u32 + 1));
                assert_eq!(option.set_size, Some(40));
            }
            // A row the menu cuts reads as the part it shows.
            assert_reads_where_it_paints(page, &options);
            options
        };
        let top = check(&page);
        assert_eq!(row_of(&page.context, page.id(), top[0].id), 0);

        // Scrolled on: rows that left the menu leave the list box, and a row
        // still shown keeps its node.
        page.context
            .update_component(page.select, |select, _| {
                select.menu_scroll.offset = Some(300.0)
            })
            .unwrap();
        page.frame(640.0, false);
        let scrolled = check(&page);
        assert_ne!(row_of(&page.context, page.id(), scrolled[0].id), 0);
        let kept = scrolled
            .iter()
            .filter(|option| top.iter().any(|before| before.id == option.id))
            .count();
        assert!(kept > 0 && kept < scrolled.len(), "{kept}");
    }

    #[test]
    fn issue200_a_select_listbox_reads_only_what_the_page_leaves_of_its_menu() {
        use crate::{NodeStyle, ScrollAxes, ScrollOffset, ScrollView, Select, SelectOption, Stack};
        // A 400x300 page that scrolls, 100 px of space, a Select, and a tall
        // block under it.
        let document = DocumentId::new(1).unwrap();
        let mut context = AppContext::new();
        context.set_reduced_motion(true);
        let mut style = NodeStyle::default();
        {
            let layout = std::sync::Arc::make_mut(&mut style.layout);
            layout.width = Some(LengthSpec::Px(400.0));
            layout.height = Some(LengthSpec::Px(300.0));
        }
        let page = context
            .create_component(document, ScrollView::new(ScrollAxes::Vertical).style(style))
            .unwrap();
        let block = |height: f32| {
            Stack::column(0.0).with_layout(move |layout| {
                layout.width = Some(LengthSpec::Fill);
                layout.height = Some(LengthSpec::Px(height));
            })
        };
        let spacer = context
            .create_detached_component(document, block(100.0))
            .unwrap();
        context.append_child(page, spacer).unwrap();
        let select = context
            .create_detached_component(
                document,
                Select::new(Some("v0")).options(
                    (0..8).map(|n| SelectOption::new(format!("v{n}"), format!("Option {n}"))),
                ),
            )
            .unwrap();
        context.append_child(page, select).unwrap();
        let tail = context
            .create_detached_component(document, block(900.0))
            .unwrap();
        context.append_child(page, tail).unwrap();
        let mut shaper = bundled_face_shaper();
        let viewport = LayoutViewport::new(400.0, 300.0);
        product_frame(&mut context, document, viewport, &mut shaper);
        assert!(context.toggle_select(select).unwrap());
        product_frame(&mut context, document, viewport, &mut shaper);
        // The page scrolls under the open menu with no layout: the menu goes
        // with its field, and the page's edge cuts its top rows away.
        assert!(
            context
                .scroll_to(page, ScrollOffset { x: 0.0, y: 180.0 })
                .unwrap()
        );
        context.rebuild_hit_test(document);
        let id = select.stable_id();
        let menu = context.open_select_menu(id).unwrap();
        let window = LayoutBox {
            x: 0.0,
            y: 0.0,
            width: 400.0,
            height: 300.0,
        };
        // Where the window shows each row: its part inside the menu, scrolled
        // with the page, inside the page.
        let shown = menu
            .options
            .iter()
            .map(|row| {
                crate::world::intersect_layout_boxes(row.bounds, menu.viewport)
                    .map(|part| LayoutBox {
                        y: part.y - 180.0,
                        ..part
                    })
                    .and_then(|part| crate::world::intersect_layout_boxes(part, window))
            })
            .collect::<Vec<_>>();
        assert!(shown[0].is_none(), "the page hides the first row: {menu:?}");
        assert!(
            shown.iter().flatten().any(|part| part.y == 0.0),
            "and cuts one: {shown:?}"
        );
        assert!(shown.iter().any(Option::is_some));
        let (_, _, options) = listbox(&spoken(&context, document), id);
        assert_eq!(
            options
                .iter()
                .map(|option| (row_of(&context, id, option.id), option.bounds))
                .collect::<Vec<_>>(),
            shown
                .iter()
                .enumerate()
                .filter_map(|(row, bounds)| Some((row, (*bounds)?)))
                .collect::<Vec<_>>()
        );
        // Each reads where the pointer picks it and hits the field.
        for option in &options {
            let row = row_of(&context, id, option.id);
            let (x, y) = centre(option.bounds);
            assert_eq!(context.world().hit_test(document, x, y), Some(id));
            let (x, y) = context.menu_point(id, x, y);
            assert_eq!(crate::select::select_option_at(&menu, x, y), Some(row));
        }
    }

    #[test]
    fn issue200_a_closed_select_has_no_listbox() {
        let mut page = SelectPage::new(1000.0);
        let listbox_id = crate::virtual_listbox_id(page.id()).unwrap();
        let options = listbox(&spoken(&page.context, page.document), page.id()).2;
        assert!(page.context.toggle_select(page.select).unwrap());
        page.frame(1000.0, false);
        let nodes = spoken(&page.context, page.document);
        let field = &nodes[&page.id()];
        assert_eq!(field.expanded, Some(false));
        assert_eq!(field.active_descendant, None);
        assert!(field.children.is_empty(), "{field:?}");
        assert!(!nodes.contains_key(&listbox_id));
        assert!(options.iter().all(|option| !nodes.contains_key(&option.id)));
        // A row id from the open menu chooses nothing now.
        let (_, slot) = crate::decode_virtual_menu_item(options[3].id).unwrap();
        assert!(
            !page
                .context
                .apply_accessibility_action(
                    page.document,
                    crate::AccessibilityActionRequest {
                        target: options[3].id,
                        action: crate::AccessibilityAction::ActivateMenuItem {
                            menu: page.id(),
                            slot,
                        },
                    },
                )
                .unwrap()
        );
        assert_eq!(
            page.context
                .read(page.select, |select| select.value.clone())
                .unwrap()
                .as_deref(),
            Some("v0")
        );
    }

    #[test]
    fn issue200_an_option_chosen_through_accessibility_is_a_press_on_its_row() {
        for action in ["activate", "click"] {
            let mut page = SelectPage::new(1000.0);
            let options = listbox(&spoken(&page.context, page.document), page.id()).2;
            let (_, slot) = crate::decode_virtual_menu_item(options[3].id).unwrap();
            assert!(
                page.context
                    .apply_accessibility_action(
                        page.document,
                        crate::AccessibilityActionRequest {
                            target: options[3].id,
                            action: if action == "click" {
                                crate::AccessibilityAction::Click
                            } else {
                                crate::AccessibilityAction::ActivateMenuItem {
                                    menu: page.id(),
                                    slot,
                                }
                            },
                        },
                    )
                    .unwrap(),
                "{action}"
            );
            assert_eq!(
                page.context
                    .read(page.select, |select| (select.value.clone(), select.opened))
                    .unwrap(),
                (Some("v3".into()), false),
                "{action}"
            );
            page.frame(1000.0, false);
            let field = &spoken(&page.context, page.document)[&page.id()];
            assert_eq!(field.expanded, Some(false));
            assert_eq!(field.value.as_deref(), Some("Option 3"));
        }
    }

    #[test]
    fn issue200_a_search_dropdown_listbox_keeps_its_options_while_the_query_filters() {
        use crate::{SearchDropdown, SearchDropdownOption, Stack};
        let document = DocumentId::new(1).unwrap();
        let mut context = AppContext::new();
        context.set_reduced_motion(true);
        let root = context
            .create_component(
                document,
                Stack::column(0.0).with_layout(|layout| {
                    layout.width = Some(LengthSpec::Px(400.0));
                }),
            )
            .unwrap();
        let field = context
            .create_detached_component(
                document,
                SearchDropdown::new(Some("cherry")).options(
                    ["apple", "apricot", "banana", "blueberry", "cherry"]
                        .map(|fruit| SearchDropdownOption::new(fruit, fruit.to_uppercase())),
                ),
            )
            .unwrap();
        context.append_child(root, field).unwrap();
        let mut shaper = bundled_face_shaper();
        let viewport = LayoutViewport::new(400.0, 600.0);
        let mut frame = |context: &mut AppContext| {
            product_frame(context, document, viewport, &mut shaper);
            context.rebuild_hit_test(document);
        };
        frame(&mut context);
        assert!(context.toggle_search_dropdown(field).unwrap());
        frame(&mut context);
        let id = field.stable_id();
        let labels = |options: &[crate::AccessibilityNode]| {
            options
                .iter()
                .map(|option| option.label.as_deref().unwrap().to_owned())
                .collect::<Vec<_>>()
        };
        let (_, _, all) = listbox(&spoken(&context, document), id);
        assert_eq!(
            labels(&all),
            ["APPLE", "APRICOT", "BANANA", "BLUEBERRY", "CHERRY"]
        );
        assert_eq!(
            all.iter().map(|option| option.selected).collect::<Vec<_>>(),
            [false, false, false, false, true].map(Some)
        );

        // The query keeps the berries and the banana: the rows it hides leave the
        // list box, and the ones it keeps keep their nodes, numbered among the
        // matches.
        context
            .update_component(field, |dropdown, _| {
                dropdown.set_query("b");
            })
            .unwrap();
        frame(&mut context);
        let (combo, listbox_node, matches) = listbox(&spoken(&context, document), id);
        assert_eq!(labels(&matches), ["BANANA", "BLUEBERRY"]);
        assert_eq!(
            matches.iter().map(|option| option.id).collect::<Vec<_>>(),
            [all[2].id, all[3].id]
        );
        assert_eq!(listbox_node.set_size, Some(2));
        assert_eq!(
            matches
                .iter()
                .map(|option| (option.position_in_set, option.set_size))
                .collect::<Vec<_>>(),
            [(Some(1), Some(2)), (Some(2), Some(2))]
        );
        // The first match takes the highlight.
        assert_eq!(combo.active_descendant, Some(all[2].id));
        let menu = context.open_select_menu(id).unwrap();
        for (row, option) in matches.iter().enumerate() {
            assert_eq!(option.bounds, menu.options[row].bounds);
            let (x, y) = centre(option.bounds);
            assert_eq!(crate::select::select_option_at(&menu, x, y), Some(row));
        }

        // Chosen through accessibility, the second match is the blueberry, not
        // the second option.
        assert!(
            context
                .apply_accessibility_action(
                    document,
                    crate::AccessibilityActionRequest {
                        target: matches[1].id,
                        action: crate::AccessibilityAction::Click,
                    },
                )
                .unwrap()
        );
        assert_eq!(
            context
                .read(field, |dropdown| dropdown.value.clone())
                .unwrap()
                .as_deref(),
            Some("blueberry")
        );
        frame(&mut context);
        let nodes = spoken(&context, document);
        assert_eq!(nodes[&id].expanded, Some(false));
        assert!(!nodes.contains_key(&crate::virtual_listbox_id(id).unwrap()));
    }

    #[test]
    fn issue200_a_multiple_dropdown_listbox_reads_its_checked_options_as_chosen() {
        use crate::{Dropdown, DropdownOption, Stack};
        let document = DocumentId::new(1).unwrap();
        let mut context = AppContext::new();
        context.set_reduced_motion(true);
        let root = context
            .create_component(
                document,
                Stack::column(0.0).with_layout(|layout| {
                    layout.width = Some(LengthSpec::Px(400.0));
                }),
            )
            .unwrap();
        let field = context
            .create_detached_component(
                document,
                Dropdown::multiple(["b"])
                    .options(["a", "b", "c"].map(|value| DropdownOption::new(value, value))),
            )
            .unwrap();
        context.append_child(root, field).unwrap();
        let mut shaper = bundled_face_shaper();
        let viewport = LayoutViewport::new(400.0, 600.0);
        product_frame(&mut context, document, viewport, &mut shaper);
        assert!(context.toggle_dropdown(field).unwrap());
        product_frame(&mut context, document, viewport, &mut shaper);
        let (_, listbox_node, options) = listbox(&spoken(&context, document), field.stable_id());
        assert!(listbox_node.multiselectable);
        assert_eq!(
            options
                .iter()
                .map(|option| option.selected)
                .collect::<Vec<_>>(),
            [false, true, false].map(Some)
        );
        // Choosing one checks it and keeps the menu open, as a press does.
        assert!(
            context
                .apply_accessibility_action(
                    document,
                    crate::AccessibilityActionRequest {
                        target: options[2].id,
                        action: crate::AccessibilityAction::Click,
                    },
                )
                .unwrap()
        );
        product_frame(&mut context, document, viewport, &mut shaper);
        let (_, _, options) = listbox(&spoken(&context, document), field.stable_id());
        assert_eq!(
            options
                .iter()
                .map(|option| option.selected)
                .collect::<Vec<_>>(),
            [false, true, true].map(Some)
        );
    }

    /// Two windows of one application, each a document with its own viewport.
    /// Each holds a `position: relative` card with a fixed box deep inside it
    /// (a quarter of the viewport wide, in its bottom-right corner), an absolute
    /// box with no positioned ancestor (the initial containing block's
    /// bottom-left corner), and a scrolled list with a fixed box inside it (the
    /// window's top-left corner).
    struct Windows {
        context: AppContext,
        shaper: NanaTextEngineShaper,
        viewports: [LayoutViewport; 2],
    }

    const WINDOWS: [DocumentId; 2] = [DocumentId::new(1).unwrap(), DocumentId::new(2).unwrap()];
    /// Each window's node ids start here.
    const WINDOW_BASE: [u64; 2] = [1, 1_000];
    /// Node ids within a window, after its document and page.
    const PINNED: u64 = 4;
    const CORNER: u64 = 5;
    const LIST: u64 = 6;
    const LIST_PIN: u64 = 8;

    fn window_node(window: usize, offset: u64) -> StableNodeId {
        crate::world::reflow_oracle::node(WINDOW_BASE[window] + offset)
    }

    fn windows_queues() -> Vec<MutationQueue> {
        use crate::InteractionState;
        use nana_ui_core::OverflowSpec;
        let mut queues = Vec::new();
        for (window, document) in WINDOWS.into_iter().enumerate() {
            let (mut b, page) = Builder::page(document, WINDOW_BASE[window], 300.0);
            let card = b.element(page, card(PositionSpec::Relative, 300.0));
            let wrapper = b.element(card, wrapper(PositionSpec::Static));
            let pinned = b.element(
                wrapper,
                LayoutStyle {
                    position: PositionSpec::Fixed,
                    offset_right: Some(LengthSpec::Px(0.0)),
                    offset_bottom: Some(LengthSpec::Px(0.0)),
                    width: Some(LengthSpec::Percent(25.0)),
                    height: Some(LengthSpec::Px(20.0)),
                    ..LayoutStyle::default()
                },
            );
            let corner = b.element(
                page,
                LayoutStyle {
                    position: PositionSpec::Absolute,
                    offset_left: Some(LengthSpec::Px(0.0)),
                    offset_bottom: Some(LengthSpec::Px(0.0)),
                    ..sized(30.0, 20.0)
                },
            );
            let list = b.element(
                page,
                LayoutStyle {
                    overflow_y: OverflowSpec::Scroll,
                    direction: Some(FlexDirection::Column),
                    ..sized(200.0, 100.0)
                },
            );
            b.element(list, sized(200.0, 400.0));
            let list_pin = b.element(
                list,
                LayoutStyle {
                    position: PositionSpec::Fixed,
                    offset_left: Some(LengthSpec::Px(0.0)),
                    offset_top: Some(LengthSpec::Px(0.0)),
                    ..sized(40.0, 10.0)
                },
            );
            assert_eq!(
                [pinned, corner, list, list_pin],
                [PINNED, CORNER, LIST, LIST_PIN].map(|offset| window_node(window, offset))
            );
            for id in [pinned, corner, list_pin] {
                b.queue.set_interaction(
                    id,
                    InteractionState {
                        pointer_events: true,
                        focusable: false,
                    },
                );
            }
            queues.push(b.queue);
        }
        queues
    }

    impl Windows {
        fn new(viewports: [LayoutViewport; 2]) -> Self {
            let mut context = AppContext::new();
            for queue in windows_queues() {
                context.commit_mutations(queue).unwrap();
            }
            let mut shaper = bundled_face_shaper();
            // A window's first frame lays out against the viewport it opens at.
            for (document, viewport) in WINDOWS.into_iter().zip(viewports) {
                resize_frame(&mut context, document, viewport, &mut shaper);
                context.rebuild_hit_test(document);
            }
            Self {
                context,
                shaper,
                viewports,
            }
        }

        fn rect(&self, window: usize, offset: u64) -> (f32, f32, f32, f32) {
            let b = self
                .context
                .world()
                .layout_box(window_node(window, offset))
                .unwrap();
            (b.x, b.y, b.width, b.height)
        }

        fn boxes(&self, window: usize) -> Vec<(StableNodeId, Option<LayoutBox>)> {
            let world = self.context.world();
            world
                .document_order(WINDOWS[window])
                .into_iter()
                .map(|id| (id, world.layout_box(id)))
                .collect()
        }

        /// Every published result of `window`, by its geometry.
        fn results(&self, window: usize) -> Vec<(StableNodeId, Option<crate::LayoutResult>)> {
            let world = self.context.world();
            world
                .document_order(WINDOWS[window])
                .into_iter()
                .map(|id| {
                    (
                        id,
                        world
                            .layout_result(id)
                            .map(|result| result.as_ref().clone()),
                    )
                })
                .collect()
        }

        /// The node hit testing gives `(x, y)` of `window`, and the bounds
        /// accessibility reads for node `offset` of it.
        fn hit_and_spoken(
            &self,
            window: usize,
            offset: u64,
            (x, y): (f32, f32),
        ) -> (u64, LayoutBox) {
            let world = self.context.world();
            let hit = world.hit_test(WINDOWS[window], x, y).unwrap();
            let id = window_node(window, offset);
            let spoken = world
                .project_accessibility_nodes_in_viewport(&[id])
                .into_iter()
                .find(|node| node.id == id)
                .unwrap()
                .bounds;
            (hit.get() - WINDOW_BASE[window], spoken)
        }

        /// Each fixed or initial-containing-block box of `window` sits where its
        /// viewport puts it, and hit testing and accessibility read that box.
        fn assert_placed(&self, window: usize) {
            let LayoutViewport { width, height } = self.viewports[window];
            let pinned = (width * 0.75, height - 20.0, width * 0.25, 20.0);
            let corner = (0.0, height - 20.0, 30.0, 20.0);
            let list_pin = (0.0, 0.0, 40.0, 10.0);
            assert_eq!(self.rect(window, PINNED), pinned, "window {window}");
            assert_eq!(self.rect(window, CORNER), corner, "window {window}");
            assert_eq!(self.rect(window, LIST_PIN), list_pin, "window {window}");
            for (offset, (x, y, w, h)) in [(PINNED, pinned), (CORNER, corner), (LIST_PIN, list_pin)]
            {
                let (hit, spoken) = self.hit_and_spoken(window, offset, (x + w / 2.0, y + h / 2.0));
                assert_eq!(hit, offset, "window {window}: hit at node {offset}");
                assert_eq!(
                    (spoken.x, spoken.y, spoken.width, spoken.height),
                    (x, y, w, h),
                    "window {window}: accessibility bounds of node {offset}"
                );
            }
        }

        fn resize(&mut self, window: usize, width: f32, height: f32) -> WorkCounters {
            self.viewports[window] = LayoutViewport::new(width, height);
            let counters = resize_frame(
                &mut self.context,
                WINDOWS[window],
                self.viewports[window],
                &mut self.shaper,
            );
            self.context.rebuild_hit_test(WINDOWS[window]);
            counters
        }
    }

    #[test]
    fn issue200_fixed_boxes_hang_from_their_own_windows_viewport() {
        let mut windows = Windows::new([
            LayoutViewport::new(800.0, 600.0),
            LayoutViewport::new(500.0, 700.0),
        ]);
        windows.assert_placed(0);
        windows.assert_placed(1);

        // The list scrolls: the fixed box inside it stays on the window, for the
        // pointer and for accessibility, as it is painted.
        let mut queue = MutationQueue::new();
        queue.set_scroll_offset(
            window_node(0, LIST),
            crate::ScrollOffset { x: 0.0, y: 50.0 },
        );
        windows.context.commit_mutations(queue).unwrap();
        windows.context.rebuild_hit_test(WINDOWS[0]);
        assert_eq!(
            windows.context.world().scroll_offset(window_node(0, LIST)),
            Some(crate::ScrollOffset { x: 0.0, y: 50.0 })
        );
        windows.assert_placed(0);

        // Each window resizes on its own: its boxes follow its viewport, and the
        // other window runs no layout pass, keeps every box and result, and
        // keeps the viewport it was laid out at.
        for (window, (width, height)) in [(0, (1000.0, 700.0)), (1, (360.0, 480.0))] {
            let other = 1 - window;
            let boxes = windows.boxes(other);
            let results = windows.results(other);
            let stats = windows.context.layout_frontier_stats(WINDOWS[other]);
            windows.resize(window, width, height);
            windows.assert_placed(window);
            assert_eq!(windows.boxes(other), boxes, "window {other}'s boxes");
            assert_eq!(windows.results(other), results, "window {other}'s results");
            assert_eq!(
                windows.context.layout_frontier_stats(WINDOWS[other]),
                stats,
                "no layout pass ran for window {other}"
            );
            assert_eq!(
                windows.context.world().document_viewport(WINDOWS[other]),
                Some(windows.viewports[other])
            );
            assert!(windows.context.take_system_work().is_empty());
            assert!(
                windows
                    .context
                    .take_layout_frontier_seeds(WINDOWS[other])
                    .is_empty()
            );
            windows.assert_placed(other);
        }

        // The same windows opened at these sizes place everything the same.
        let mut cold = Windows::new(windows.viewports);
        let mut queue = MutationQueue::new();
        queue.set_scroll_offset(
            window_node(0, LIST),
            crate::ScrollOffset { x: 0.0, y: 50.0 },
        );
        cold.context.commit_mutations(queue).unwrap();
        for document in WINDOWS {
            assert_matches_cold(&mut windows.context, &mut cold.context, document);
        }
    }

    #[test]
    fn issue200_an_edit_pending_in_one_window_waits_for_that_windows_frame() {
        let mut windows = Windows::new([
            LayoutViewport::new(800.0, 600.0),
            LayoutViewport::new(500.0, 700.0),
        ]);
        let grow = |windows: &mut Windows, width: f32| {
            let mut queue = MutationQueue::new();
            queue.set_style(
                window_node(1, CORNER),
                styled(LayoutStyle {
                    position: PositionSpec::Absolute,
                    offset_left: Some(LengthSpec::Px(0.0)),
                    offset_bottom: Some(LengthSpec::Px(0.0)),
                    ..sized(width, 20.0)
                }),
            );
            windows.context.commit_mutations(queue).unwrap();
        };
        let draw = |windows: &mut Windows, window: usize| {
            product_frame(
                &mut windows.context,
                WINDOWS[window],
                windows.viewports[window],
                &mut windows.shaper,
            );
        };
        let before = windows.boxes(0);
        // Window 1's corner grows; window 0 draws its next frame first. It has
        // nothing of its own to lay out, and lays out nothing for window 1.
        grow(&mut windows, 50.0);
        let passes = windows.context.layout_invocations();
        draw(&mut windows, 0);
        assert_eq!(
            windows.context.layout_invocations(),
            passes,
            "no pass for window 1's edit"
        );
        assert_eq!(windows.boxes(0), before);
        assert_eq!(
            windows.rect(1, CORNER).2,
            30.0,
            "window 1 has not drawn yet"
        );
        // Window 1's own frame lays the edit out.
        draw(&mut windows, 1);
        assert_eq!(windows.rect(1, CORNER), (0.0, 680.0, 50.0, 20.0));
        assert!(windows.context.take_system_work().is_empty());

        // A host that drains the whole context and hands window 0's pass every
        // seed: the pass gives window 1's back rather than dropping them.
        grow(&mut windows, 70.0);
        let seeds = windows.context.take_system_work().layout_frontier_seeds;
        assert!(!seeds.is_empty());
        windows
            .context
            .layout_document_with_frontier(WINDOWS[0], windows.viewports[0], &seeds)
            .unwrap();
        assert_eq!(windows.boxes(0), before);
        draw(&mut windows, 1);
        assert_eq!(windows.rect(1, CORNER), (0.0, 680.0, 70.0, 20.0));
        assert!(windows.context.take_system_work().is_empty());
    }

    /// A chain of absolute boxes that fill the viewport, then a static box whose
    /// absolute children hang from the box above it: the plan of the static box
    /// holds them as remote overlays.
    fn remote_chain(with_children: bool) -> Tree {
        let fill = |inset: f32| LayoutStyle {
            position: PositionSpec::Absolute,
            offset_left: Some(LengthSpec::Px(inset)),
            offset_top: Some(LengthSpec::Px(inset)),
            offset_right: Some(LengthSpec::Px(inset)),
            offset_bottom: Some(LengthSpec::Px(inset)),
            direction: Some(FlexDirection::Column),
            ..LayoutStyle::default()
        };
        let mut spec = vec![
            (None, fill(0.0)),
            (Some(0), fill(8.0)),
            (Some(1), fill(0.0)),
            (
                Some(2),
                LayoutStyle {
                    direction: Some(FlexDirection::Column),
                    ..LayoutStyle::default()
                },
            ),
        ];
        if with_children {
            spec.push((Some(3), badge()));
            spec.push((
                Some(3),
                LayoutStyle {
                    position: PositionSpec::Absolute,
                    offset_left: Some(LengthSpec::Px(0.0)),
                    offset_bottom: Some(LengthSpec::Px(0.0)),
                    width: Some(LengthSpec::Percent(100.0)),
                    height: Some(LengthSpec::Px(20.0)),
                    ..LayoutStyle::default()
                },
            ));
        }
        Tree::new(spec)
    }

    #[test]
    fn issue200_removed_remote_children_leave_the_walk() {
        // Removing the children while the window resizes left them in the static
        // box's plan; every round of the remote walk found them moved and placed
        // nothing, so layout never returned.
        let mut tree = remote_chain(true);
        tree.resize(810.0, 600.0);
        let mut queue = MutationQueue::new();
        queue.despawn_subtree(tree.ids[4]);
        queue.despawn_subtree(tree.ids[5]);
        tree.context.commit_mutations(queue).unwrap();
        tree.spec.truncate(4);
        tree.ids.truncate(4);
        for (width, height) in [(830.0, 610.0), (850.0, 620.0), (800.0, 600.0)] {
            tree.resize(width, height);
        }
        let mut cold = remote_chain(false);
        cold.resize(800.0, 600.0);
        for index in 0..4 {
            assert_eq!(tree.rect(index), cold.rect(index));
        }
    }
}

mod scroll_clip_chain {
    //! Scroll is a behavior of the same retained formatting context.

    use crate::world::reflow_oracle::{bundled_face_shaper, product_frame};
    use crate::{
        AppContext, Button, DocumentId, GpuTextureView, InlineContent, InlineFlow, LayoutViewport,
        MutationQueue, NodeStyle, ParticipationKind, ScrollAxes, ScrollOffset, ScrollView,
    };
    use nana_ui_core::{
        DisplaySpec, FlexDirection, GridTrack, LayoutStyle, LengthSpec, ScrollbarVisibility,
    };
    use std::sync::Arc;

    #[test]
    fn issue201_scroll_is_orthogonal_to_flow_flex_grid_and_inline_contexts() {
        for display in [DisplaySpec::Block, DisplaySpec::Flex, DisplaySpec::Grid] {
            let document = DocumentId::new(1).unwrap();
            let mut cx = AppContext::new();
            let mut shaper = bundled_face_shaper();
            let viewport = LayoutViewport::new(400.0, 300.0);
            let scroll = cx
                .create_component(
                    document,
                    ScrollView::new(ScrollAxes::Both)
                        .scrollbars(ScrollbarVisibility::Hidden)
                        .with_layout(|style| {
                            style.display = Some(display);
                            style.direction = Some(FlexDirection::Row);
                            style.width = Some(LengthSpec::Px(160.0));
                            style.height = Some(LengthSpec::Px(60.0));
                            style.grid_columns = Some(vec![GridTrack::Px(80.0); 2]);
                        }),
                )
                .unwrap();
            for _ in 0..20 {
                let layout = LayoutStyle {
                    display: Some(DisplaySpec::InlineBlock),
                    width: Some(LengthSpec::Px(80.0)),
                    height: Some(LengthSpec::Px(24.0)),
                    flex_shrink: Some(0.0),
                    ..Default::default()
                };
                let child = cx
                    .create_detached_component(
                        document,
                        Button::new("Item").layout(Arc::new(layout)),
                    )
                    .unwrap();
                cx.append_child(scroll, child).unwrap();
            }
            product_frame(&mut cx, document, viewport, &mut shaper);
            let world = cx.world();
            let generation = world.layout_generation();
            let context = world.layout_node(scroll.stable_id()).unwrap().established;
            let boxes: Vec<_> = world
                .document_order(document)
                .into_iter()
                .map(|id| (id, world.layout_box(id)))
                .collect();
            let mut q = MutationQueue::new();
            q.set_scroll_offset(scroll.stable_id(), ScrollOffset { x: 20.0, y: 20.0 });
            cx.commit_mutations(q).unwrap();
            let work = product_frame(&mut cx, document, viewport, &mut shaper);
            assert_eq!(
                (
                    work.layout_measure_nodes,
                    work.layout_placement_nodes,
                    work.text_shaped,
                    work.entities_spawned,
                    work.entities_despawned
                ),
                (0, 0, 0, 0, 0),
                "{display:?}"
            );
            assert_eq!(cx.world().layout_generation(), generation);
            assert_eq!(
                cx.world()
                    .layout_node(scroll.stable_id())
                    .unwrap()
                    .established,
                context
            );
            for (id, bounds) in boxes {
                assert_eq!(cx.world().layout_box(id), bounds);
            }
            let offset = cx.world().scroll_offset(scroll.stable_id()).unwrap();
            assert!(
                offset.x > 0.0 || offset.y > 0.0,
                "fixture must really scroll in {display:?}"
            );
        }
    }

    /// A native mixed paragraph and a host-texture custom renderer under one
    /// scroller: scrolling lays nothing out, keeps the renderer's descriptor, and
    /// moves hit testing and accessibility together by the offset.
    #[test]
    fn issue201_a_paragraph_and_a_custom_renderer_scroll_without_layout() {
        let document = DocumentId::new(1).unwrap();
        let mut cx = AppContext::new();
        let mut shaper = bundled_face_shaper();
        let viewport = LayoutViewport::new(400.0, 300.0);
        let scroll = cx
            .create_component(
                document,
                ScrollView::new(ScrollAxes::Vertical)
                    .scrollbars(ScrollbarVisibility::Hidden)
                    .with_layout(|style| {
                        style.display = Some(DisplaySpec::Block);
                        style.width = Some(LengthSpec::Px(300.0));
                        style.height = Some(LengthSpec::Px(100.0));
                    }),
            )
            .unwrap();
        let fixed = |width: f32, height: f32| {
            Arc::new(LayoutStyle {
                width: Some(LengthSpec::Px(width)),
                height: Some(LengthSpec::Px(height)),
                ..Default::default()
            })
        };
        let atom = cx
            .create_detached_component(document, Button::new("Go").layout(fixed(40.0, 20.0)))
            .unwrap();
        let paragraph = cx
            .create_detached_component(
                document,
                InlineFlow::new(
                    InlineContent::builder()
                        .text("Status ")
                        .child(atom.stable_id())
                        .text(" and a line of text long enough to wrap in the scroller")
                        .build(),
                )
                .width(LengthSpec::Px(280.0)),
            )
            .unwrap();
        let texture = cx
            .create_detached_component(
                document,
                GpuTextureView::new("preview")
                    .label("Preview")
                    .with_pointer_events(true)
                    .style(NodeStyle {
                        layout: fixed(200.0, 160.0),
                        ..Default::default()
                    }),
            )
            .unwrap();
        cx.append_child(scroll, paragraph).unwrap();
        cx.append_child(scroll, texture).unwrap();
        product_frame(&mut cx, document, viewport, &mut shaper);
        cx.rebuild_hit_test(document);

        let world = cx.world();
        assert_eq!(
            world.layout_node(atom.stable_id()).unwrap().participation,
            Some(ParticipationKind::AtomicInline)
        );
        let render = world.custom_render(texture.stable_id()).cloned().unwrap();
        let generation = world.layout_generation();
        let boxes: Vec<_> = world
            .document_order(document)
            .into_iter()
            .map(|id| (id, world.layout_box(id)))
            .collect();
        let texture_box = world.layout_box(texture.stable_id()).unwrap();
        // The content overflows the 100px viewport; scroll most of the way.
        assert!(
            texture_box.y + texture_box.height > 180.0,
            "{texture_box:?}"
        );
        let offset = 80.0;

        let mut q = MutationQueue::new();
        q.set_scroll_offset(scroll.stable_id(), ScrollOffset { x: 0.0, y: offset });
        cx.commit_mutations(q).unwrap();
        let work = product_frame(&mut cx, document, viewport, &mut shaper);
        assert_eq!(
            (
                work.layout_measure_nodes,
                work.layout_placement_nodes,
                work.text_shaped,
                work.entities_spawned,
                work.entities_despawned
            ),
            (0, 0, 0, 0, 0)
        );
        // The hit index takes the scroll as an in-place patch, not a rebuild.
        for (scroller, delta) in cx.take_scroll_hit_updates() {
            cx.update_hit_test_scroll(document, scroller, delta);
        }
        let world = cx.world();
        assert_eq!(world.layout_generation(), generation);
        for (id, bounds) in boxes {
            assert_eq!(world.layout_box(id), bounds);
        }
        assert_eq!(world.custom_render(texture.stable_id()), Some(&render));
        assert_eq!(world.scroll_offset(scroll.stable_id()).unwrap().y, offset);

        // Hit testing and accessibility both see the texture where it scrolled to.
        let moved_y = texture_box.y - offset;
        // A point inside the viewport, on the part of the texture still shown.
        let center = (texture_box.x + 20.0, moved_y + 60.0);
        assert!((0.0..100.0).contains(&center.1));
        assert_eq!(
            world.hit_test(document, center.0, center.1),
            Some(texture.stable_id())
        );
        let accessible = world
            .project_accessibility_in_viewport(document)
            .into_iter()
            .find(|node| node.id == texture.stable_id())
            .unwrap()
            .bounds;
        assert!((accessible.y - moved_y).abs() < 0.01, "{accessible:?}");
        // The paragraph's atom scrolled up out of the viewport with it: its old
        // place no longer reaches it.
        let atom_box = world.layout_box(atom.stable_id()).unwrap();
        assert!(atom_box.y + atom_box.height < offset, "{atom_box:?}");
        assert_ne!(
            world.hit_test(
                document,
                atom_box.x + atom_box.width / 2.0,
                atom_box.y + atom_box.height / 2.0
            ),
            Some(atom.stable_id())
        );
    }

    /// The clip chain (#201): a page that is a containing block holds a static
    /// scroller `outer`. In it, a positioned card holds a static scroller
    /// `inner` with a row in flow, an absolute box whose block is the card, and
    /// a fixed box; after the card, a static clipping box `side` holds an
    /// absolute box whose block is the page.
    mod clip_chain {
        use crate::world::reflow_oracle::{
            Builder, accessible_frame, assert_matches_cold, bundled_face_shaper, column, fixed,
            product_frame,
        };
        use crate::{
            AppContext, DocumentId, InteractionState, LayoutBox, LayoutViewport, MutationQueue,
            ScrollOffset, StableNodeId,
        };
        use nana_ui_core::{FlexDirection, LayoutStyle, LengthSpec, OverflowSpec, PositionSpec};

        pub(super) const DOCUMENT: u64 = 1;
        const VIEWPORT: (f32, f32) = (800.0, 600.0);

        #[derive(Clone, Copy)]
        pub(super) struct Ids {
            pub(super) page: StableNodeId,
            pub(super) outer: StableNodeId,
            pub(super) card: StableNodeId,
            pub(super) inner: StableNodeId,
            pub(super) row: StableNodeId,
            pub(super) escaping: StableNodeId,
            pub(super) fixed: StableNodeId,
            pub(super) side: StableNodeId,
            pub(super) escaping_far: StableNodeId,
        }

        pub(super) fn document() -> DocumentId {
            DocumentId::new(DOCUMENT).unwrap()
        }

        fn viewport() -> LayoutViewport {
            LayoutViewport::new(VIEWPORT.0, VIEWPORT.1)
        }

        pub(super) fn scroller(width: f32, height: f32, overflow: OverflowSpec) -> LayoutStyle {
            LayoutStyle {
                overflow_x: overflow,
                overflow_y: overflow,
                direction: Some(FlexDirection::Column),
                flex_shrink: Some(0.0),
                ..fixed(width, height)
            }
        }

        pub(super) fn card(position: PositionSpec) -> LayoutStyle {
            LayoutStyle {
                position,
                flex_shrink: Some(0.0),
                ..column(Some(300.0))
            }
        }

        fn placed(position: PositionSpec, [left, top, width, height]: [f32; 4]) -> LayoutStyle {
            LayoutStyle {
                position,
                offset_left: Some(LengthSpec::Px(left)),
                offset_top: Some(LengthSpec::Px(top)),
                ..fixed(width, height)
            }
        }

        /// The fixture, laid out with `card` as the card's position and `inner`
        /// as the inner scroller's overflow.
        pub(super) fn build(card_position: PositionSpec, inner: OverflowSpec) -> (AppContext, Ids) {
            let (mut b, page) = Builder::page(document(), 1, 400.0);
            b.queue.set_style(
                page,
                crate::world::reflow_oracle::styled(LayoutStyle {
                    position: PositionSpec::Relative,
                    ..column(Some(400.0))
                }),
            );
            let outer = b.element(page, scroller(300.0, 200.0, OverflowSpec::Auto));
            let card_id = b.element(outer, card(card_position));
            b.element(card_id, fixed(300.0, 50.0));
            let inner_id = b.element(card_id, scroller(200.0, 100.0, inner));
            let row = b.element(inner_id, fixed(100.0, 30.0));
            b.element(inner_id, fixed(200.0, 400.0));
            let escaping = b.element(
                inner_id,
                placed(PositionSpec::Absolute, [150.0, 60.0, 80.0, 30.0]),
            );
            let fixed_id = b.element(
                inner_id,
                placed(PositionSpec::Fixed, [320.0, 10.0, 30.0, 30.0]),
            );
            let side = b.element(outer, scroller(300.0, 60.0, OverflowSpec::Hidden));
            let escaping_far = b.element(
                side,
                placed(PositionSpec::Absolute, [250.0, 150.0, 100.0, 40.0]),
            );
            b.element(outer, fixed(300.0, 500.0));
            for id in [row, escaping, fixed_id, escaping_far] {
                b.queue.set_interaction(
                    id,
                    InteractionState {
                        pointer_events: true,
                        focusable: false,
                    },
                );
            }
            let mut context = AppContext::new();
            context.commit_mutations(b.queue).unwrap();
            let mut shaper = bundled_face_shaper();
            product_frame(&mut context, document(), viewport(), &mut shaper);
            context.rebuild_hit_test(document());
            (
                context,
                Ids {
                    page,
                    outer,
                    card: card_id,
                    inner: inner_id,
                    row,
                    escaping,
                    fixed: fixed_id,
                    side,
                    escaping_far,
                },
            )
        }

        fn rect(x: f32, y: f32, width: f32, height: f32) -> LayoutBox {
            LayoutBox {
                x,
                y,
                width,
                height,
            }
        }

        fn accessible(context: &AppContext, id: StableNodeId) -> (LayoutBox, Option<LayoutBox>) {
            let node = context
                .world()
                .project_accessibility_in_viewport(document())
                .into_iter()
                .find(|node| node.id == id)
                .unwrap_or_else(|| panic!("{id:?} is in the accessible tree"));
            (node.bounds, node.clip)
        }

        fn close(a: LayoutBox, b: LayoutBox) -> bool {
            (a.x - b.x).abs() < 0.01
                && (a.y - b.y).abs() < 0.01
                && (a.width - b.width).abs() < 0.01
                && (a.height - b.height).abs() < 0.01
        }

        #[test]
        fn issue201_the_clip_chain_follows_containing_blocks() {
            let (context, ids) = build(PositionSpec::Relative, OverflowSpec::Auto);
            let world = context.world();
            // The fixture is where the gates below assume it is.
            for (id, expected) in [
                (ids.outer, rect(0.0, 0.0, 300.0, 200.0)),
                (ids.inner, rect(0.0, 50.0, 200.0, 100.0)),
                (ids.row, rect(0.0, 50.0, 100.0, 30.0)),
                (ids.escaping, rect(150.0, 60.0, 80.0, 30.0)),
                (ids.fixed, rect(320.0, 10.0, 30.0, 30.0)),
                (ids.side, rect(0.0, 150.0, 300.0, 60.0)),
                (ids.escaping_far, rect(250.0, 150.0, 100.0, 40.0)),
            ] {
                assert_eq!(world.layout_box(id), Some(expected), "box of {id:?}");
            }
            let anchors = |id| {
                let result = world.layout_result(id).unwrap();
                (result.clip, result.clip_parent)
            };
            // In flow: the nearest overflow container, through the parent.
            assert_eq!(anchors(ids.row), (Some(ids.inner), Some(ids.inner)));
            assert_eq!(
                world.layout_clip_chain(ids.row).collect::<Vec<_>>(),
                [ids.inner, ids.outer]
            );
            // The card is the block: the inner scroller does not reach the box,
            // the outer one does.
            assert_eq!(anchors(ids.escaping), (Some(ids.outer), Some(ids.card)));
            assert_eq!(
                world.layout_clip_chain(ids.escaping).collect::<Vec<_>>(),
                [ids.outer]
            );
            // The page is the block: neither `side` nor `outer` reaches it.
            assert_eq!(anchors(ids.escaping_far), (None, Some(ids.page)));
            assert_eq!(world.layout_clip_chain(ids.escaping_far).count(), 0);
            // Fixed: nothing above it.
            assert_eq!(anchors(ids.fixed), (None, None));
            assert_eq!(world.layout_clip_chain(ids.fixed).count(), 0);
        }

        /// One scroll position's worth of expectations: where each box sits in
        /// the window and what its clip chain leaves of it.
        fn assert_clipped_and_scrolled(context: &AppContext, ids: Ids, inner_y: f32, outer_y: f32) {
            let at = format!("inner {inner_y}, outer {outer_y}");
            let outer_clip = rect(0.0, 0.0, 300.0, 200.0);
            // The row scrolls with both and is cut by both.
            let inner_top = 50.0 - outer_y;
            let (bounds, clip) = accessible(context, ids.row);
            assert!(
                close(bounds, rect(0.0, 50.0 - inner_y - outer_y, 100.0, 30.0)),
                "{at}: row {bounds:?}"
            );
            let expected_inner = {
                let top = inner_top.max(0.0);
                rect(0.0, top, 200.0, (inner_top + 100.0).min(200.0) - top)
            };
            assert!(
                clip.is_some_and(|clip| close(clip, expected_inner)),
                "{at}: row clip {clip:?}, expected {expected_inner:?}"
            );
            // The box hanging from the card scrolls with the outer scroller
            // only, and only the outer one cuts it.
            let (bounds, clip) = accessible(context, ids.escaping);
            assert!(
                close(bounds, rect(150.0, 60.0 - outer_y, 80.0, 30.0)),
                "{at}: escaping {bounds:?}"
            );
            assert_eq!(clip, Some(outer_clip), "{at}: escaping clip");
            // The boxes hanging from the page and the window: no scroll, no clip.
            let (bounds, clip) = accessible(context, ids.escaping_far);
            assert_eq!(
                (bounds, clip),
                (rect(250.0, 150.0, 100.0, 40.0), None),
                "{at}: far"
            );
            let (bounds, clip) = accessible(context, ids.fixed);
            assert_eq!(
                (bounds, clip),
                (rect(320.0, 10.0, 30.0, 30.0), None),
                "{at}: fixed"
            );

            let world = context.world();
            let hit = |x: f32, y: f32| world.hit_test(document(), x, y);
            // Past the inner scroller's right edge, still inside the outer one:
            // the escaping box takes the pointer there.
            let y = 60.0 - outer_y + 15.0;
            if (0.0..200.0).contains(&y) {
                assert_eq!(
                    hit(215.0, y),
                    Some(ids.escaping),
                    "{at}: escaping beyond inner"
                );
                assert_eq!(
                    hit(160.0, y),
                    Some(ids.escaping),
                    "{at}: escaping over inner"
                );
            }
            // Its unscrolled place is not it, once the outer scroller moved it
            // off: the inner scroller's scroll alone never moves it.
            if outer_y >= 30.0 {
                assert_ne!(hit(215.0, 85.0), Some(ids.escaping), "{at}: old place");
            }
            // Past the outer scroller's right edge and below `side`'s bottom.
            assert_eq!(
                hit(330.0, 170.0),
                Some(ids.escaping_far),
                "{at}: far, right"
            );
            assert_eq!(hit(310.0, 188.0), Some(ids.escaping_far), "{at}: far, low");
            assert_eq!(hit(335.0, 25.0), Some(ids.fixed), "{at}: fixed");
            // The row, where it scrolled to, while the inner scroller shows it.
            let row_y = 50.0 - inner_y - outer_y + 15.0;
            if row_y > inner_top && row_y < inner_top + 100.0 && row_y > 0.0 {
                assert_eq!(hit(50.0, row_y), Some(ids.row), "{at}: row");
            }
        }

        #[test]
        fn issue201_scroll_reaches_a_box_only_through_its_containing_block() {
            let (mut context, ids) = build(PositionSpec::Relative, OverflowSpec::Auto);
            let mut shaper = bundled_face_shaper();
            let generation = context.world().layout_generation();
            assert_clipped_and_scrolled(&context, ids, 0.0, 0.0);
            for (inner_y, outer_y) in [(40.0, 0.0), (40.0, 30.0), (0.0, 30.0), (25.0, 70.0)] {
                let mut q = MutationQueue::new();
                q.set_scroll_offset(ids.inner, ScrollOffset { x: 0.0, y: inner_y });
                q.set_scroll_offset(ids.outer, ScrollOffset { x: 0.0, y: outer_y });
                context.commit_mutations(q).unwrap();
                let work = product_frame(&mut context, document(), viewport(), &mut shaper);
                assert_eq!(
                    (work.layout_measure_nodes, work.layout_placement_nodes),
                    (0, 0),
                    "scrolling lays nothing out"
                );
                assert_eq!(context.world().layout_generation(), generation);
                assert_eq!(context.world().scroll_offset(ids.inner).unwrap().y, inner_y);
                assert_eq!(context.world().scroll_offset(ids.outer).unwrap().y, outer_y);
                // The hit index takes the scroll in place; the escaped entries do
                // not hang under the scrollers they escape, so it leaves them.
                for (scroller, delta) in context.take_scroll_hit_updates() {
                    context.update_hit_test_scroll(document(), scroller, delta);
                }
                assert_clipped_and_scrolled(&context, ids, inner_y, outer_y);
                // A rebuilt index answers the same.
                context.rebuild_hit_test(document());
                assert_clipped_and_scrolled(&context, ids, inner_y, outer_y);
            }
        }

        /// The room an open select menu has (the window, less the clips above
        /// its field, in the field's layout px) reads the same chain.
        #[test]
        fn issue201_a_menu_room_is_cut_by_the_clip_chain_alone() {
            let (mut context, ids) = build(PositionSpec::Relative, OverflowSpec::Auto);
            let mut shaper = bundled_face_shaper();
            let mut q = MutationQueue::new();
            q.set_scroll_offset(ids.inner, ScrollOffset { x: 0.0, y: 40.0 });
            q.set_scroll_offset(ids.outer, ScrollOffset { x: 0.0, y: 30.0 });
            context.commit_mutations(q).unwrap();
            product_frame(&mut context, document(), viewport(), &mut shaper);
            let room = |id| {
                let room = context.world().select_menu_room(id).unwrap();
                (room.left, room.top, room.right, room.bottom)
            };
            // Both scrollers cut the row, each where the scroll above it puts it.
            assert_eq!(room(ids.row), (0.0, 90.0, 200.0, 190.0));
            // Only the outer one cuts the card's absolute box.
            assert_eq!(room(ids.escaping), (0.0, 30.0, 300.0, 230.0));
            // Nothing cuts the page's absolute box but the window.
            assert_eq!(room(ids.escaping_far), (0.0, 0.0, 800.0, 600.0));
        }

        #[test]
        fn issue201_a_moving_clip_chain_matches_a_cold_layout() {
            let (mut context, ids) = build(PositionSpec::Relative, OverflowSpec::Auto);
            let mut shaper = bundled_face_shaper();
            let mut scroll = MutationQueue::new();
            scroll.set_scroll_offset(ids.outer, ScrollOffset { x: 0.0, y: 30.0 });
            context.commit_mutations(scroll).unwrap();
            product_frame(&mut context, document(), viewport(), &mut shaper);
            for (position, inner) in [
                // The card stops being a block: the box now escapes both
                // scrollers and hangs from the page.
                (PositionSpec::Static, OverflowSpec::Auto),
                // The inner box stops clipping: nothing between to escape but
                // the outer scroller.
                (PositionSpec::Static, OverflowSpec::Visible),
                // And back.
                (PositionSpec::Relative, OverflowSpec::Visible),
                (PositionSpec::Relative, OverflowSpec::Auto),
            ] {
                let mut q = MutationQueue::new();
                q.set_style(
                    ids.card,
                    crate::world::reflow_oracle::styled(card(position)),
                );
                q.set_style(
                    ids.inner,
                    crate::world::reflow_oracle::styled(scroller(200.0, 100.0, inner)),
                );
                context.commit_mutations(q).unwrap();
                product_frame(&mut context, document(), viewport(), &mut shaper);
                let (mut cold, _) = build(position, inner);
                let mut q = MutationQueue::new();
                q.set_scroll_offset(ids.outer, ScrollOffset { x: 0.0, y: 30.0 });
                cold.commit_mutations(q).unwrap();
                product_frame(&mut cold, document(), viewport(), &mut shaper);
                assert_matches_cold(&mut context, &mut cold, document());
                let escaping = context.world().layout_result(ids.escaping).unwrap();
                let expected = match (position, inner) {
                    (PositionSpec::Relative, OverflowSpec::Auto) => {
                        (Some(ids.outer), Some(ids.card))
                    }
                    (PositionSpec::Relative, _) => (Some(ids.outer), Some(ids.inner)),
                    // Hangs from the page: the parent of the outer scroller.
                    _ => (None, Some(ids.page)),
                };
                assert_eq!(
                    (escaping.clip, escaping.clip_parent),
                    expected,
                    "{position:?} {inner:?}"
                );
            }
        }

        /// One frame as `RuntimeDocument` runs it, hit index included: a
        /// scroll-only frame patches the scrollers in place, anything else
        /// patches the dirty branches.
        fn hosted_frame(context: &mut AppContext) {
            let mut shaper = bundled_face_shaper();
            let work = accessible_frame(context, document(), viewport(), &mut shaper, false);
            let updates = context.take_scroll_hit_updates();
            let mut dirty = work.input_hit_test.clone();
            dirty.sort_unstable();
            dirty.dedup();
            if dirty.is_empty() {
                return;
            }
            if context.hit_test_work_is_scroll_only(&dirty, &updates) {
                for (scroller, delta) in updates {
                    context.update_hit_test_scroll(document(), scroller, delta);
                }
            } else {
                context.rebuild_hit_test_for(document(), &dirty);
            }
        }

        /// What the pointer finds on a grid over the page: the topmost hit and
        /// every candidate under it.
        fn hit_grid(context: &AppContext) -> Vec<(Option<StableNodeId>, Vec<StableNodeId>)> {
            let mut hits = Vec::new();
            for y in (0..30).map(|row| row as f32 * 8.0 + 1.0) {
                for x in (0..48).map(|column| column as f32 * 8.0 + 1.0) {
                    let world = context.world();
                    hits.push((
                        world.hit_test(document(), x, y),
                        world.hit_test_candidates(document(), x, y),
                    ));
                }
            }
            hits
        }

        /// The retained index answers as a rebuilt one does, then is rebuilt.
        fn assert_patched_hits_match_rebuild(context: &mut AppContext, at: &str) {
            let patched = hit_grid(context);
            context.rebuild_hit_test(document());
            assert_eq!(patched, hit_grid(context), "{at}: patched hit index");
        }

        /// Scoped patches move escaped entries with the chain: scroll patches
        /// first shift the scrollers, then the chain changes under them (the
        /// box stops escaping, escapes further, starts again).
        #[test]
        fn issue201_a_patched_hit_index_follows_the_moving_chain() {
            let (mut context, ids) = build(PositionSpec::Relative, OverflowSpec::Auto);
            // A fixed box in a patched branch spans two projection trees and
            // sends the patch to a full rebuild; without it the branches patch.
            let mut q = MutationQueue::new();
            q.despawn_subtree(ids.fixed);
            q.set_scroll_offset(ids.inner, ScrollOffset { x: 0.0, y: 20.0 });
            context.commit_mutations(q).unwrap();
            hosted_frame(&mut context);
            assert_patched_hits_match_rebuild(&mut context, "inner scrolled");
            let mut outer_y = 30.0;
            for (position, inner) in [
                (PositionSpec::Static, OverflowSpec::Auto),
                (PositionSpec::Static, OverflowSpec::Visible),
                (PositionSpec::Relative, OverflowSpec::Visible),
                (PositionSpec::Relative, OverflowSpec::Auto),
                (PositionSpec::Relative, OverflowSpec::Visible),
                (PositionSpec::Static, OverflowSpec::Auto),
            ] {
                // A scroll the index takes in place, so the scrollers' shifts
                // are live when the chain changes.
                let mut q = MutationQueue::new();
                q.set_scroll_offset(ids.outer, ScrollOffset { x: 0.0, y: outer_y });
                context.commit_mutations(q).unwrap();
                let mut shaper = bundled_face_shaper();
                product_frame(&mut context, document(), viewport(), &mut shaper);
                for (scroller, delta) in context.take_scroll_hit_updates() {
                    context.update_hit_test_scroll(document(), scroller, delta);
                }
                outer_y = if outer_y == 30.0 { 45.0 } else { 30.0 };
                let mut q = MutationQueue::new();
                q.set_style(
                    ids.card,
                    crate::world::reflow_oracle::styled(card(position)),
                );
                q.set_style(
                    ids.inner,
                    crate::world::reflow_oracle::styled(scroller(200.0, 100.0, inner)),
                );
                context.commit_mutations(q).unwrap();
                hosted_frame(&mut context);
                assert_patched_hits_match_rebuild(&mut context, &format!("{position:?} {inner:?}"));
            }
        }

        /// The ancestors an escaped box passes over still lend it their
        /// transforms, logical and presented, but not their scroll.
        #[test]
        fn issue201_an_escaped_box_takes_the_transforms_it_passes_over() {
            use nana_ui_core::PaintTransform;
            let (mut context, ids) = build(PositionSpec::Relative, OverflowSpec::Auto);
            let shifted = |e: f32| LayoutStyle {
                transform: Some(PaintTransform {
                    e,
                    ..PaintTransform::default()
                }),
                ..scroller(200.0, 100.0, OverflowSpec::Auto)
            };
            let mut q = MutationQueue::new();
            q.set_scroll_offset(ids.inner, ScrollOffset { x: 0.0, y: 40.0 });
            q.set_style(
                ids.inner,
                crate::world::reflow_oracle::styled(shifted(20.0)),
            );
            context.commit_mutations(q).unwrap();
            hosted_frame(&mut context);
            context.rebuild_hit_test(document());
            let expected = rect(170.0, 60.0, 80.0, 30.0);
            assert_eq!(accessible(&context, ids.escaping).0, expected);
            assert_eq!(
                context.world().presentation_input_bounds(ids.escaping),
                Some(expected),
                "the input projection reads the same chain"
            );
            assert_eq!(
                context.world().hit_test(document(), 245.0, 75.0),
                Some(ids.escaping)
            );

            // A compositor overlay moves the scroller: the box moves with what
            // is presented, for the pointer and for accessibility.
            let mut q = MutationQueue::new();
            q.set_style(
                ids.inner,
                crate::world::reflow_oracle::styled(shifted(60.0)),
            );
            q.start_animation(
                crate::AnimationSpec::new(
                    crate::AnimationId::new(1).unwrap(),
                    ids.inner,
                    std::time::Duration::ZERO,
                    std::time::Duration::from_millis(100),
                    std::time::Duration::from_millis(16),
                    crate::Easing::Linear,
                )
                .with_property(crate::AnimatableProperty::Transform)
                .with_range(
                    crate::MotionValue::Transform(PaintTransform {
                        e: 20.0,
                        ..PaintTransform::default()
                    }),
                    crate::MotionTo::Value(crate::MotionValue::Transform(PaintTransform {
                        e: 60.0,
                        ..PaintTransform::default()
                    })),
                ),
            );
            context.commit_mutations(q).unwrap();
            hosted_frame(&mut context);
            context.advance_animations(std::time::Duration::from_millis(50));
            context.rebuild_hit_test(document());
            let presented = rect(190.0, 60.0, 80.0, 30.0);
            assert!(
                close(accessible(&context, ids.escaping).0, presented),
                "{:?}",
                accessible(&context, ids.escaping).0
            );
            assert_eq!(
                context.world().hit_test(document(), 265.0, 75.0),
                Some(ids.escaping)
            );
        }

        /// An escaped entry's place among its clip parent's entries is not its
        /// paint order: a box later in the tree paints over it, and takes the
        /// pointer there, however the index sorted them.
        #[test]
        fn issue201_an_escaped_box_is_ranked_by_paint_order() {
            let (mut context, ids) = build(PositionSpec::Relative, OverflowSpec::Auto);
            let document_node = context.world().parent_id(ids.page).unwrap();
            // The escaping box last in its scroller: its sibling order, 3, is
            // past the cover's, 2, in the card.
            let mut q = MutationQueue::new();
            q.insert(ids.inner, ids.escaping, None);
            let cover = StableNodeId::new(100).unwrap();
            q.create(
                cover,
                document(),
                crate::NodeKind::Element { tag: "div".into() },
            );
            q.insert(ids.card, cover, None);
            q.set_style(
                cover,
                crate::world::reflow_oracle::styled(LayoutStyle {
                    transform: Some(nana_ui_core::PaintTransform {
                        f: -90.0,
                        ..nana_ui_core::PaintTransform::default()
                    }),
                    ..fixed(300.0, 30.0)
                }),
            );
            // A z-index elsewhere sorts every sibling group by (z, order).
            let raised = StableNodeId::new(101).unwrap();
            q.create(
                raised,
                document(),
                crate::NodeKind::Element { tag: "div".into() },
            );
            q.insert(document_node, raised, None);
            q.set_style(
                raised,
                crate::world::reflow_oracle::styled(LayoutStyle {
                    z_index: Some(1),
                    ..fixed(10.0, 10.0)
                }),
            );
            for id in [cover, raised] {
                q.set_interaction(
                    id,
                    InteractionState {
                        pointer_events: true,
                        focusable: false,
                    },
                );
            }
            context.commit_mutations(q).unwrap();
            hosted_frame(&mut context);
            context.rebuild_hit_test(document());
            assert_eq!(
                context.world().layout_box(cover).map(|b| b.y),
                Some(150.0),
                "the cover follows the scroller in flow"
            );
            // The cover's presented box is 60..90, over the escaping box.
            assert_eq!(
                context.world().hit_test(document(), 215.0, 75.0),
                Some(cover)
            );
            assert_eq!(
                context
                    .world()
                    .hit_test_candidates(document(), 215.0, 75.0)
                    .first(),
                Some(&cover)
            );
            // Off the cover, the escaping box still takes the pointer: only the
            // outer scroller cuts it.
            let mut q = MutationQueue::new();
            q.set_style(
                cover,
                crate::world::reflow_oracle::styled(fixed(300.0, 30.0)),
            );
            context.commit_mutations(q).unwrap();
            hosted_frame(&mut context);
            assert_eq!(
                context.world().hit_test(document(), 215.0, 75.0),
                Some(ids.escaping)
            );
        }
    }

    /// The scrollable range follows the clip chain (#201): a scroller's range
    /// counts a box only when the scroller reaches it, as CSS scrollable
    /// overflow does (CSS Overflow 3: a box contributes to the scrollable
    /// overflow of its containing block chain). A relative scrolling page holds
    /// a static wrapper around a static scroller `holder`; in `holder` an
    /// in-flow block, a relative block with an absolute box far below it, an
    /// absolute box `far` and a `fixed` box `pinned` (holding an absolute box of
    /// its own), the last two far outside `holder`.
    mod scroll_range {
        use crate::world::reflow_oracle::{
            Builder, assert_matches_cold, bundled_face_shaper, column, fixed, product_frame, styled,
        };
        use crate::{
            AppContext, DocumentId, LayoutViewport, MutationQueue, ScrollOffset, StableNodeId,
        };
        use nana_ui_core::{LayoutStyle, LengthSpec, OverflowSpec, PositionSpec};

        fn document() -> DocumentId {
            DocumentId::new(1).unwrap()
        }

        fn viewport() -> LayoutViewport {
            LayoutViewport::new(800.0, 600.0)
        }

        #[derive(Clone, Copy)]
        struct Ids {
            page: StableNodeId,
            holder: StableNodeId,
            far: StableNodeId,
            pinned: StableNodeId,
        }

        /// The positions a state gives `holder`, `far` and `pinned`.
        type State = (PositionSpec, PositionSpec, PositionSpec);

        fn scroller(position: PositionSpec, width: f32, height: f32) -> LayoutStyle {
            LayoutStyle {
                position,
                overflow_x: OverflowSpec::Auto,
                overflow_y: OverflowSpec::Auto,
                height: Some(LengthSpec::Px(height)),
                flex_shrink: Some(0.0),
                ..column(Some(width))
            }
        }

        fn placed(position: PositionSpec, [left, top, width, height]: [f32; 4]) -> LayoutStyle {
            LayoutStyle {
                position,
                offset_left: Some(LengthSpec::Px(left)),
                offset_top: Some(LengthSpec::Px(top)),
                ..fixed(width, height)
            }
        }

        fn holder_style(position: PositionSpec) -> LayoutStyle {
            scroller(position, 300.0, 200.0)
        }

        fn far_style(position: PositionSpec) -> LayoutStyle {
            placed(position, [50.0, 900.0, 40.0, 40.0])
        }

        fn pinned_style(position: PositionSpec) -> LayoutStyle {
            placed(position, [600.0, 1200.0, 40.0, 40.0])
        }

        fn build((holder, far, pinned): State) -> (AppContext, Ids) {
            let (mut b, page) = Builder::page(document(), 1, 800.0);
            b.queue
                .set_style(page, styled(scroller(PositionSpec::Relative, 800.0, 600.0)));
            let wrap = b.element(
                page,
                LayoutStyle {
                    flex_shrink: Some(0.0),
                    ..column(Some(300.0))
                },
            );
            let holder_id = b.element(wrap, holder_style(holder));
            b.element(holder_id, fixed(300.0, 250.0));
            let near = b.element(
                holder_id,
                LayoutStyle {
                    position: PositionSpec::Relative,
                    flex_shrink: Some(0.0),
                    ..fixed(100.0, 50.0)
                },
            );
            b.element(
                near,
                placed(PositionSpec::Absolute, [0.0, 400.0, 40.0, 40.0]),
            );
            let far_id = b.element(holder_id, far_style(far));
            let pinned_id = b.element(holder_id, pinned_style(pinned));
            b.element(
                pinned_id,
                placed(PositionSpec::Absolute, [0.0, 100.0, 40.0, 40.0]),
            );
            let mut context = AppContext::new();
            context.commit_mutations(b.queue).unwrap();
            let mut shaper = bundled_face_shaper();
            product_frame(&mut context, document(), viewport(), &mut shaper);
            (
                context,
                Ids {
                    page,
                    holder: holder_id,
                    far: far_id,
                    pinned: pinned_id,
                },
            )
        }

        /// How far each scroller's range reaches down: `holder`'s and the
        /// page's, as their scroll metrics, published results and clamping all
        /// say it.
        fn reach(context: &AppContext, ids: Ids) -> (f32, f32) {
            let world = context.world();
            let bottom = |id: StableNodeId| {
                let metrics = world.scroll_metrics(id).expect("a scroller has metrics");
                let result = world.layout_result(id).expect("a laid-out scroller");
                let bottom = result.bounds.y + metrics.content_height;
                assert_eq!(
                    result.scroll_extent.y + result.scroll_extent.height,
                    bottom,
                    "{id:?}: the published scroll extent is the measured range"
                );
                assert_eq!(
                    result.overflow.y + result.overflow.height,
                    bottom,
                    "{id:?}: the published overflow is the measured range"
                );
                bottom
            };
            (bottom(ids.holder), bottom(ids.page))
        }

        /// Where the ranges reach in `state`. The relative box's absolute box
        /// reaches 690 inside `holder` whatever happens; `far` reaches 940 and
        /// `pinned` (with its absolute box) 1340 in whichever scroller reaches
        /// them; the page's own box ends at 600.
        fn expected((holder, far, pinned): State) -> (f32, f32) {
            let mut ranges = (690.0_f32, 600.0_f32);
            let mut count = |position: PositionSpec, bottom: f32| match position {
                PositionSpec::Fixed => {}
                PositionSpec::Absolute if holder == PositionSpec::Static => {
                    ranges.1 = ranges.1.max(bottom)
                }
                _ => ranges.0 = ranges.0.max(bottom),
            };
            count(far, 940.0);
            count(pinned, 1340.0);
            ranges
        }

        fn offsets(context: &mut AppContext, ids: Ids) {
            let mut q = MutationQueue::new();
            q.set_scroll_offset(
                ids.holder,
                ScrollOffset {
                    x: 0.0,
                    y: 10_000.0,
                },
            );
            q.set_scroll_offset(
                ids.page,
                ScrollOffset {
                    x: 0.0,
                    y: 10_000.0,
                },
            );
            context.commit_mutations(q).unwrap();
            let mut shaper = bundled_face_shaper();
            product_frame(context, document(), viewport(), &mut shaper);
        }

        #[test]
        fn issue201_boxes_a_scroller_does_not_reach_stay_out_of_its_range() {
            let state = (
                PositionSpec::Static,
                PositionSpec::Absolute,
                PositionSpec::Fixed,
            );
            let (mut context, ids) = build(state);
            let world = context.world();
            // The chain the range follows: `far` escapes `holder` and hangs
            // from the wrapper, inside the page; `pinned` hangs from nothing.
            assert_eq!(
                world.layout_clip_chain(ids.far).collect::<Vec<_>>(),
                [ids.page]
            );
            assert_eq!(world.layout_clip_chain(ids.pinned).count(), 0);
            assert_eq!(world.layout_box(ids.far).map(|b| b.y), Some(900.0));
            assert_eq!(world.layout_box(ids.pinned).map(|b| b.y), Some(1200.0));
            // `holder` ends at its relative box's absolute box; the page, which
            // reaches `far`, at `far`; nothing at `pinned`.
            assert_eq!(reach(&context, ids), (690.0, 940.0));
            assert_eq!(reach(&context, ids), expected(state));
            // Clamping reads the same range.
            offsets(&mut context, ids);
            let world = context.world();
            assert_eq!(world.scroll_offset(ids.holder).unwrap().y, 490.0);
            assert_eq!(world.scroll_offset(ids.page).unwrap().y, 340.0);
        }

        /// Changing what the chain says moves the ranges with it, incrementally
        /// as a cold layout of the same inputs has them: `holder` turning
        /// relative takes `far` in; `far` turning fixed leaves every range;
        /// `pinned` turning absolute enters the page's.
        #[test]
        fn issue201_a_moving_chain_moves_the_ranges_as_a_cold_layout_has_them() {
            use PositionSpec::{Absolute, Fixed, Relative, Static};
            let mut state = (Static, Absolute, Fixed);
            let (mut context, ids) = build(state);
            offsets(&mut context, ids);
            for next in [
                (Relative, Absolute, Fixed),
                (Static, Absolute, Fixed),
                (Static, Fixed, Fixed),
                (Static, Fixed, Absolute),
                (Relative, Fixed, Absolute),
                (Static, Absolute, Fixed),
            ] {
                state = next;
                let mut q = MutationQueue::new();
                q.set_style(ids.holder, styled(holder_style(state.0)));
                q.set_style(ids.far, styled(far_style(state.1)));
                q.set_style(ids.pinned, styled(pinned_style(state.2)));
                context.commit_mutations(q).unwrap();
                let mut shaper = bundled_face_shaper();
                product_frame(&mut context, document(), viewport(), &mut shaper);
                offsets(&mut context, ids);
                let (mut cold, _) = build(state);
                offsets(&mut cold, ids);
                assert_eq!(reach(&context, ids), expected(state), "{state:?}");
                assert_eq!(reach(&cold, ids), expected(state), "cold {state:?}");
                for id in [ids.holder, ids.page] {
                    assert_eq!(
                        context.world().scroll_metrics(id),
                        cold.world().scroll_metrics(id),
                        "{state:?}: metrics of {id:?}"
                    );
                    assert_eq!(
                        context.world().scroll_offset(id),
                        cold.world().scroll_offset(id),
                        "{state:?}: offset of {id:?}"
                    );
                }
                assert_matches_cold(&mut context, &mut cold, document());
            }
        }

        /// Scrolling either scroller lays nothing out and leaves both ranges
        /// where they were.
        #[test]
        fn issue201_scrolling_leaves_the_ranges_and_lays_nothing_out() {
            let state = (
                PositionSpec::Static,
                PositionSpec::Absolute,
                PositionSpec::Fixed,
            );
            let (mut context, ids) = build(state);
            let generation = context.world().layout_generation();
            let mut shaper = bundled_face_shaper();
            for (holder_y, page_y) in [(100.0, 0.0), (100.0, 200.0), (0.0, 340.0), (490.0, 10.0)] {
                let mut q = MutationQueue::new();
                q.set_scroll_offset(
                    ids.holder,
                    ScrollOffset {
                        x: 0.0,
                        y: holder_y,
                    },
                );
                q.set_scroll_offset(ids.page, ScrollOffset { x: 0.0, y: page_y });
                context.commit_mutations(q).unwrap();
                let work = product_frame(&mut context, document(), viewport(), &mut shaper);
                assert_eq!(
                    (
                        work.layout_measure_nodes,
                        work.layout_placement_nodes,
                        work.text_shaped
                    ),
                    (0, 0, 0),
                    "scrolling lays nothing out"
                );
                let world = context.world();
                assert_eq!(world.layout_generation(), generation);
                assert_eq!(world.scroll_offset(ids.holder).unwrap().y, holder_y);
                assert_eq!(world.scroll_offset(ids.page).unwrap().y, page_y);
                assert_eq!(reach(&context, ids), expected(state));
            }
        }
    }
}

mod intrinsic_sizing {
    //! Issue #202: replaced content is one kind of layout item in every context.
    //!
    //! - Gate A: an image with no size of its own takes the natural size its
    //!   resource reports, and one with only a width takes its height from the
    //!   aspect ratio, as a flex item, a grid item, an atomic inline box and an
    //!   absolutely positioned box alike. Each matches a cold layout.
    //! - Gate B: replaced content aligns by its bottom edge: as an atom of a
    //!   native paragraph (its margin box's, Gate G) and under `align-items:
    //!   baseline` (its border box's), an image's and a custom renderer's bottom
    //!   edge sits on the text's baseline.
    //! - Gate C: an image, a video, a host texture, a GPU canvas and a bare
    //!   custom-render anchor are one kind of layout item: built the way an
    //!   application builds each, they are replaced content with the same
    //!   participation in every context, and the same natural size gives each
    //!   the same box. Each matches a cold layout.
    //! - Gate D: min/max bound a natural size without bending its aspect ratio
    //!   (CSS 2.1 §10.4), with no size, or one side, of the box's own.
    //! - Gate E: invalidation follows the natural size and nothing else. The
    //!   same size reported again, and a switch to another resource of the same
    //!   size, lay nothing out; a switch to one of another size lays out the box
    //!   that reads it.
    //! - Gate F: markup gives a video and a host texture their fit the way it
    //!   gives an image its fit: the cascaded `object-fit`, every keyword of it,
    //!   ahead of the `fit` attribute. The painter then sizes all three with
    //!   `ContentFit::fitted_size` (nana-ui `scene_paint/replaced_context_tests.rs`
    //!   checks its `url()` path against the host-texture one).
    //! - Gate G: atoms sharing a line of an ordinary block align by baseline
    //!   (CSS 2.1 §10.8.1, `vertical-align: baseline`). An inline-block with no
    //!   line box, one whose overflow is not visible and replaced content sit on
    //!   the baseline by their bottom margin edge; an inline-block with text by
    //!   its last line. The line box spans the highest top to the lowest bottom.
    //!   A taller atom, text that wraps to another line in a box of the same
    //!   size, and a line box arriving in an empty atom each place the line
    //!   again, and each matches a cold layout.
    //! - Gate H: an inline-block whose line boxes all lie in its descendants
    //!   sits by the last of them, where it sits inside the atom: below a block,
    //!   the last of a wrapped label's lines, the second of two stacked atoms,
    //!   inside a fixed block. Changes that move it place the line again; one
    //!   that does not moves nothing. Each matches a cold layout.
    //! - Gate I: a line box of an ordinary block starts from the block's strut:
    //!   its face's descent below an empty atom, half the leading on each side,
    //!   none on a block-level box's line.

    use nana_ui_core::{
        AlignSpec, DisplaySpec, FlexDirection, GridTrack, LayoutStyle, LengthSpec, PositionSpec,
    };

    use crate::world::reflow_oracle::{
        Builder, assert_matches_cold, bundled_face_shaper, product_frame, styled,
    };
    use crate::world::tests::replaced_media::{image_layout, url};
    use std::sync::Arc;

    use crate::world::{DocumentId, StableNodeId, UiWorld};
    use crate::{
        AppContext, CustomRenderNode, GPU_VIEW_RENDERER, GpuTextureView, GpuView,
        HOST_TEXTURE_RENDERER, InlineContent, InlineFlow, LayoutContentKind, LayoutViewport,
        MutationQueue, NanaTextEngineShaper, ParticipationKind as Part, ReplacedMetadata,
        ReplacedResource, Video,
    };

    fn viewport() -> LayoutViewport {
        LayoutViewport::new(800.0, 600.0)
    }

    /// How a context's container and its replaced children are styled.
    #[derive(Debug, Clone, Copy)]
    enum Context {
        Flex,
        /// A flex row that stretches its items down the line.
        FlexStretch,
        Grid,
        /// A grid that stretches its items down their areas.
        GridStretch,
        Inline,
        Absolute,
    }

    impl Context {
        fn container(self) -> LayoutStyle {
            let mut layout = LayoutStyle {
                width: Some(LengthSpec::Px(600.0)),
                ..LayoutStyle::default()
            };
            match self {
                Self::Flex => {
                    layout.display = Some(DisplaySpec::Flex);
                    layout.direction = Some(FlexDirection::Row);
                }
                Self::FlexStretch => {
                    layout.display = Some(DisplaySpec::Flex);
                    layout.direction = Some(FlexDirection::Row);
                    layout.align_items = AlignSpec::Stretch;
                }
                Self::Grid | Self::GridStretch => {
                    layout.display = Some(DisplaySpec::Grid);
                    layout.grid_columns = Some(vec![GridTrack::Auto, GridTrack::Auto]);
                    if matches!(self, Self::GridStretch) {
                        layout.align_items = AlignSpec::Stretch;
                    }
                }
                Self::Inline => layout.display = Some(DisplaySpec::Block),
                Self::Absolute => {
                    layout.display = Some(DisplaySpec::Block);
                    layout.position = PositionSpec::Relative;
                    layout.height = Some(LengthSpec::Px(300.0));
                }
            }
            layout
        }

        fn item(self, mut layout: LayoutStyle, offset: f32) -> LayoutStyle {
            match self {
                Self::Flex | Self::FlexStretch | Self::Grid | Self::GridStretch => {}
                Self::Inline => layout.display = Some(DisplaySpec::InlineBlock),
                Self::Absolute => {
                    layout.position = PositionSpec::Absolute;
                    layout.offset_top = Some(LengthSpec::Px(offset));
                    layout.offset_left = Some(LengthSpec::Px(offset));
                }
            }
            layout
        }

        fn participation(self) -> Part {
            match self {
                Self::Flex | Self::FlexStretch => Part::FlexItem,
                Self::Grid | Self::GridStretch => Part::GridItem,
                Self::Inline => Part::AtomicInline,
                Self::Absolute => Part::Absolute,
            }
        }
    }

    struct Page {
        context: AppContext,
        document: DocumentId,
        shaper: NanaTextEngineShaper,
        /// Shows `pic` with no size of its own.
        auto: StableNodeId,
        /// Shows `pic` 60 wide; its height follows the aspect ratio.
        wide: StableNodeId,
    }

    impl Page {
        fn new(context: Context, reported: bool) -> Self {
            let document = DocumentId::new(1).unwrap();
            let (mut b, page) = Builder::page(document, 1, 800.0);
            let container = b.element(page, context.container());
            let auto = b.element(
                container,
                context.item(image_layout("pic", None, None), 0.0),
            );
            let wide = b.element(
                container,
                context.item(image_layout("pic", Some(60.0), None), 100.0),
            );
            if reported {
                b.queue
                    .set_replaced_metadata(url("pic"), Some(ReplacedMetadata::new(120.0, 80.0)));
            }
            let mut app = AppContext::new();
            app.commit_mutations(b.queue).unwrap();
            let mut page = Self {
                context: app,
                document,
                shaper: bundled_face_shaper(),
                auto,
                wide,
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

        fn size(&self, id: StableNodeId) -> (f32, f32) {
            let bounds = self.world().layout_box(id).unwrap();
            (bounds.width, bounds.height)
        }
    }

    /// Gate A.
    #[test]
    fn issue202_a_natural_size_reaches_every_context() {
        for context in [
            Context::Flex,
            Context::FlexStretch,
            Context::Grid,
            Context::GridStretch,
            Context::Inline,
            Context::Absolute,
        ] {
            let mut page = Page::new(context, false);
            for id in [page.auto, page.wide] {
                assert_eq!(
                    page.world().layout_node(id).unwrap().participation,
                    Some(context.participation()),
                    "{context:?}"
                );
            }
            let mut queue = MutationQueue::new();
            queue.set_replaced_metadata(url("pic"), Some(ReplacedMetadata::new(120.0, 80.0)));
            page.context.commit_mutations(queue).unwrap();
            let work = page.frame();
            assert_eq!(work.replaced_layout_seeds, 2, "{context:?}");
            assert_eq!(work.layout_full_document_fallbacks, 0, "{context:?}");
            assert_eq!(page.size(page.auto), (120.0, 80.0), "{context:?}");
            assert_eq!(page.size(page.wide), (60.0, 40.0), "{context:?}");
            let mut cold = Page::new(context, true);
            assert_matches_cold(&mut page.context, &mut cold.context, page.document);
        }
    }

    /// Gate B.
    #[test]
    fn issue202_replaced_content_sits_on_the_text_baseline_by_its_bottom_edge() {
        let document = DocumentId::new(1).unwrap();
        let (mut b, page) = Builder::page(document, 1, 800.0);
        let fixed = |width: f32, height: f32| LayoutStyle {
            width: Some(LengthSpec::Px(width)),
            height: Some(LengthSpec::Px(height)),
            ..LayoutStyle::default()
        };
        let render_anchor = |b: &mut Builder, id| {
            b.queue.set_custom_render(
                id,
                Some(CustomRenderNode::new("fixture-renderer", "anchor", 0)),
            );
        };
        // A flex row aligning by baseline.
        let row = b.element(
            page,
            LayoutStyle {
                display: Some(DisplaySpec::Flex),
                direction: Some(FlexDirection::Row),
                align_items: AlignSpec::Baseline,
                width: Some(LengthSpec::Px(600.0)),
                ..LayoutStyle::default()
            },
        );
        let row_text = b.label(row, "Baseline");
        let row_image = b.element(row, image_layout("tall", Some(20.0), Some(48.0)));
        let row_render = b.element(row, fixed(30.0, 36.0));
        render_anchor(&mut b, row_render);
        // The same two as atoms of a native paragraph.
        let line_image = b.detached(image_layout("tall", Some(20.0), Some(48.0)));
        let line_render = b.detached(fixed(30.0, 36.0));
        render_anchor(&mut b, line_render);
        let mut context = AppContext::new();
        context.commit_mutations(b.queue).unwrap();
        let paragraph = context
            .create_component(
                document,
                InlineFlow::new(
                    InlineContent::builder()
                        .text("Baseline ")
                        .child(line_image)
                        .text(" and ")
                        .child(line_render)
                        .build(),
                )
                .width(LengthSpec::Px(600.0)),
            )
            .unwrap()
            .stable_id();
        let mut shaper = bundled_face_shaper();
        product_frame(&mut context, document, viewport(), &mut shaper);
        let world = context.world();
        // Text baselines are in document space.
        let baseline = |id| world.layout_result(id).unwrap().first_baseline.unwrap();
        for (text, replaced) in [
            (row_text, [row_image, row_render]),
            (paragraph, [line_image, line_render]),
        ] {
            let text_baseline = baseline(text);
            for id in replaced {
                assert_eq!(
                    world.layout_node(id).unwrap().content,
                    crate::LayoutContentKind::Replaced
                );
                let bounds = world.layout_box(id).unwrap();
                let bottom = bounds.y + bounds.height;
                assert!(
                    (bottom - text_baseline).abs() < 0.01,
                    "{id:?} bottom {bottom} vs text baseline {text_baseline}"
                );
            }
        }
        assert_eq!(
            world.layout_node(line_image).unwrap().participation,
            Some(Part::AtomicInline)
        );
    }

    /// The replaced kinds the issue names.
    #[derive(Debug, Clone, Copy)]
    enum Kind {
        Image,
        Video,
        HostTexture,
        /// A GPU canvas: a `GpuView` render target.
        Canvas,
        /// A custom renderer's presentation anchor on a plain element.
        Anchor,
    }

    const KINDS: [Kind; 5] = [
        Kind::Image,
        Kind::Video,
        Kind::HostTexture,
        Kind::Canvas,
        Kind::Anchor,
    ];

    impl Kind {
        fn resource(self) -> ReplacedResource {
            let render = |renderer: &str, resource: &str| ReplacedResource::Render {
                renderer: Arc::from(renderer),
                resource: Arc::from(resource),
            };
            match self {
                Self::Image => url("pic"),
                Self::Video => render(HOST_TEXTURE_RENDERER, "video:1"),
                Self::HostTexture => render(HOST_TEXTURE_RENDERER, "slot:7"),
                Self::Canvas => render(GPU_VIEW_RENDERER, "3"),
                Self::Anchor => render("fixture.anchor", "anchor"),
            }
        }

        /// Adds the node under `parent` as an application would, with `layout`:
        /// an image is a style, an anchor a custom render node on an element,
        /// and the rest are their components.
        fn build(
            self,
            b: &mut Builder,
            components: &mut Vec<(StableNodeId, Self, LayoutStyle)>,
            parent: StableNodeId,
            layout: LayoutStyle,
        ) -> Option<StableNodeId> {
            match self {
                Self::Image => {
                    let mut layout = layout;
                    layout.paint.content_image =
                        Some(nana_ui_core::BackgroundImage::url("asset://pic.png"));
                    Some(b.element(parent, layout))
                }
                Self::Anchor => {
                    let id = b.element(parent, layout);
                    b.queue.set_custom_render(
                        id,
                        Some(CustomRenderNode::new("fixture.anchor", "anchor", 0)),
                    );
                    Some(id)
                }
                Self::Video | Self::HostTexture | Self::Canvas => {
                    components.push((parent, self, layout));
                    None
                }
            }
        }

        fn create(
            self,
            context: &mut AppContext,
            document: DocumentId,
            layout: LayoutStyle,
        ) -> StableNodeId {
            // The components fill their box unless told otherwise; `auto` asks
            // for the natural size, as an image's absent size does.
            let mut layout = layout;
            layout.width.get_or_insert(LengthSpec::Auto);
            layout.height.get_or_insert(LengthSpec::Auto);
            let style = styled(layout);
            match self {
                Self::Video => context
                    .create_component(document, Video::new("video:1").style(style))
                    .map(|entity| entity.stable_id()),
                Self::HostTexture => context
                    .create_component(document, GpuTextureView::new("slot:7").style(style))
                    .map(|entity| entity.stable_id()),
                Self::Canvas => context
                    .create_component(document, GpuView::new(3).style(style))
                    .map(|entity| entity.stable_id()),
                Self::Image | Self::Anchor => unreachable!("built as elements"),
            }
            .unwrap()
        }
    }

    /// Every kind under one container of `context`.
    struct Gallery {
        context: AppContext,
        document: DocumentId,
        shaper: NanaTextEngineShaper,
        nodes: Vec<(Kind, StableNodeId)>,
    }

    impl Gallery {
        fn new(context: Context, reported: Option<ReplacedMetadata>) -> Self {
            let document = DocumentId::new(1).unwrap();
            let (mut b, page) = Builder::page(document, 1, 800.0);
            let container = b.element(page, context.container());
            if let Some(metadata) = reported {
                for kind in KINDS {
                    b.queue
                        .set_replaced_metadata(kind.resource(), Some(metadata));
                }
            }
            let mut nodes = Vec::new();
            let mut components = Vec::new();
            for (index, kind) in KINDS.into_iter().enumerate() {
                let layout = context.item(LayoutStyle::default(), 130.0 * index as f32);
                if let Some(id) = kind.build(&mut b, &mut components, container, layout) {
                    nodes.push((kind, id));
                }
            }
            let mut app = AppContext::new();
            app.commit_mutations(b.queue).unwrap();
            for (parent, kind, layout) in components {
                let id = kind.create(&mut app, document, layout);
                let mut queue = MutationQueue::new();
                queue.insert(parent, id, None);
                app.commit_mutations(queue).unwrap();
                nodes.push((kind, id));
            }
            let mut gallery = Self {
                context: app,
                document,
                shaper: bundled_face_shaper(),
                nodes,
            };
            gallery.frame();
            gallery
        }

        fn frame(&mut self) -> nana_ui_core::WorkCounters {
            product_frame(
                &mut self.context,
                self.document,
                viewport(),
                &mut self.shaper,
            )
        }
    }

    /// Gate C.
    #[test]
    fn issue202_c_every_replaced_kind_is_one_layout_item() {
        for context in [
            Context::Flex,
            Context::FlexStretch,
            Context::Grid,
            Context::GridStretch,
            Context::Inline,
            Context::Absolute,
        ] {
            let mut gallery = Gallery::new(context, None);
            let world = gallery.context.world();
            for &(kind, id) in &gallery.nodes {
                assert!(
                    world.shows_replaced(&kind.resource()),
                    "{context:?} {kind:?}"
                );
                let node = world.layout_node(id).unwrap();
                assert_eq!(
                    node.content,
                    LayoutContentKind::Replaced,
                    "{context:?} {kind:?}"
                );
                assert_eq!(
                    node.participation,
                    Some(context.participation()),
                    "{context:?} {kind:?}"
                );
            }
            let mut queue = MutationQueue::new();
            for kind in KINDS {
                queue.set_replaced_metadata(
                    kind.resource(),
                    Some(ReplacedMetadata::new(120.0, 80.0)),
                );
            }
            gallery.context.commit_mutations(queue).unwrap();
            let work = gallery.frame();
            assert_eq!(work.replaced_layout_seeds, KINDS.len(), "{context:?}");
            assert_eq!(work.layout_full_document_fallbacks, 0, "{context:?}");
            for &(kind, id) in &gallery.nodes {
                let bounds = gallery.context.world().layout_box(id).unwrap();
                assert_eq!(
                    (bounds.width, bounds.height),
                    (120.0, 80.0),
                    "{context:?} {kind:?}"
                );
            }
            let mut cold = Gallery::new(context, Some(ReplacedMetadata::new(120.0, 80.0)));
            assert_matches_cold(&mut gallery.context, &mut cold.context, gallery.document);
        }
    }

    /// Gate D.
    #[test]
    fn issue202_d_min_max_keep_the_natural_ratio() {
        type Edit = fn(&mut LayoutStyle);
        let cases: [(&str, Edit, (f32, f32)); 8] = [
            (
                "max-width",
                |l| l.max_width = Some(LengthSpec::Px(60.0)),
                (60.0, 40.0),
            ),
            (
                "min-width",
                |l| l.min_width = Some(LengthSpec::Px(240.0)),
                (240.0, 160.0),
            ),
            (
                "max-height",
                |l| l.max_height = Some(LengthSpec::Px(40.0)),
                (60.0, 40.0),
            ),
            (
                "min-height",
                |l| l.min_height = Some(LengthSpec::Px(160.0)),
                (240.0, 160.0),
            ),
            // Too wide and too tall: the tighter limit wins.
            (
                "max-width and max-height",
                |l| {
                    l.max_width = Some(LengthSpec::Px(90.0));
                    l.max_height = Some(LengthSpec::Px(40.0));
                },
                (60.0, 40.0),
            ),
            // Limits that cannot both keep the ratio: each holds, the ratio gives.
            (
                "min-width and max-height",
                |l| {
                    l.min_width = Some(LengthSpec::Px(240.0));
                    l.max_height = Some(LengthSpec::Px(40.0));
                },
                (240.0, 40.0),
            ),
            // One side of its own: the other follows, then meets its own limits.
            (
                "width and max-height",
                |l| {
                    l.width = Some(LengthSpec::Px(60.0));
                    l.max_height = Some(LengthSpec::Px(30.0));
                },
                (60.0, 30.0),
            ),
            (
                "height and min-width",
                |l| {
                    l.height = Some(LengthSpec::Px(40.0));
                    l.min_width = Some(LengthSpec::Px(90.0));
                },
                (90.0, 40.0),
            ),
        ];
        for context in [
            Context::Flex,
            Context::Grid,
            Context::Inline,
            Context::Absolute,
        ] {
            for (label, edit, expected) in cases {
                let build = |reported: bool| {
                    let document = DocumentId::new(1).unwrap();
                    let (mut b, page) = Builder::page(document, 1, 800.0);
                    let container = b.element(page, context.container());
                    let mut layout = image_layout("pic", None, None);
                    edit(&mut layout);
                    let id = b.element(container, context.item(layout, 0.0));
                    if reported {
                        b.queue.set_replaced_metadata(
                            url("pic"),
                            Some(ReplacedMetadata::new(120.0, 80.0)),
                        );
                    }
                    let mut app = AppContext::new();
                    app.commit_mutations(b.queue).unwrap();
                    product_frame(&mut app, document, viewport(), &mut bundled_face_shaper());
                    (app, document, id)
                };
                let (mut warm, document, id) = build(false);
                let mut queue = MutationQueue::new();
                queue.set_replaced_metadata(url("pic"), Some(ReplacedMetadata::new(120.0, 80.0)));
                warm.commit_mutations(queue).unwrap();
                product_frame(&mut warm, document, viewport(), &mut bundled_face_shaper());
                let bounds = warm.world().layout_box(id).unwrap();
                assert_eq!(
                    (bounds.width, bounds.height),
                    expected,
                    "{context:?} {label}"
                );
                let (mut cold, _, _) = build(true);
                assert_matches_cold(&mut warm, &mut cold, document);
            }
        }
    }

    /// Gate E.
    #[test]
    fn issue202_e_only_a_new_natural_size_lays_out() {
        let mut page = Page::new(Context::Flex, true);
        let (auto, wide) = (page.auto, page.wide);
        let boxes = |page: &Page| (page.size(auto), page.size(wide));
        let before = boxes(&page);
        // A frame with nothing to do reports the last frame's counters again.
        let idle = page.frame();

        // The same size again is nothing: no update, no notice, no paint.
        let mut queue = MutationQueue::new();
        queue.set_replaced_metadata(url("pic"), Some(ReplacedMetadata::new(120.0, 80.0)));
        page.context.commit_mutations(queue).unwrap();
        assert!(page.context.take_system_work().is_empty());
        assert_eq!(page.frame(), idle);

        // Another resource of the same size: both boxes show it, neither moves.
        let mut queue = MutationQueue::new();
        queue.set_replaced_metadata(url("twin"), Some(ReplacedMetadata::new(120.0, 80.0)));
        for (id, width) in [(auto, None), (wide, Some(60.0))] {
            queue.set_style(
                id,
                styled(Context::Flex.item(image_layout("twin", width, None), 0.0)),
            );
        }
        page.context.commit_mutations(queue).unwrap();
        let work = page.frame();
        assert_eq!(work.replaced_layout_seeds, 0);
        assert_eq!(work.layout_frontier_seeds, 0);
        assert_eq!(work.layout_measure_nodes, 0);
        assert_eq!(work.intrinsic_generation_bumps, 0);
        assert!(page.world().shows_replaced(&url("twin")));
        assert!(!page.world().shows_replaced(&url("pic")));
        assert_eq!(boxes(&page), before);

        // One of another size: the two boxes that read it lay out again.
        let mut queue = MutationQueue::new();
        queue.set_replaced_metadata(url("tall"), Some(ReplacedMetadata::new(80.0, 120.0)));
        for (id, width) in [(auto, None), (wide, Some(60.0))] {
            queue.set_style(
                id,
                styled(Context::Flex.item(image_layout("tall", width, None), 0.0)),
            );
        }
        page.context.commit_mutations(queue).unwrap();
        let work = page.frame();
        assert_eq!(work.replaced_layout_seeds, 2);
        assert_eq!(boxes(&page), ((80.0, 120.0), (60.0, 90.0)));
    }

    /// Gate F.
    #[test]
    fn issue202_f_markup_fits_every_replaced_kind_alike() {
        use crate::component_registry::{ComponentTypeId, RegisterableComponent, SemanticSpec};
        use nana_ui_core::{BackgroundImageFit, ContentFit};

        let fits = |object_fit: Option<BackgroundImageFit>, attr: Option<&str>| {
            let mut layout = LayoutStyle::default();
            layout.paint.object_fit = object_fit;
            let layout = Arc::new(layout);
            let attrs: Vec<(&str, &str)> = [("data-nana-video", "1"), ("data-nana-gpu", "slot:7")]
                .into_iter()
                .chain(attr.map(|value| ("fit", value)))
                .collect();
            let video_type = ComponentTypeId::new("nana.video").unwrap();
            let mut spec = SemanticSpec::from_parts(&video_type, &layout);
            spec.attrs = &attrs;
            let video = Video::from_semantic(&spec).fit;
            let texture_type = ComponentTypeId::new("nana.gpu").unwrap();
            let mut spec = SemanticSpec::from_parts(&texture_type, &layout);
            spec.attrs = &attrs;
            let texture = GpuTextureView::from_semantic(&spec).fit;
            assert_eq!(video, texture);
            video
        };
        for fit in [
            ContentFit::Fill,
            ContentFit::Contain,
            ContentFit::Cover,
            ContentFit::None,
            ContentFit::ScaleDown,
        ] {
            // An image's `object-fit` is the same value.
            let image = BackgroundImageFit::from(fit);
            assert_eq!(image.object_fit(), Some(fit));
            // The stylesheet's `object-fit`, over any attribute.
            assert_eq!(fits(Some(image), None), fit, "{fit:?}");
            assert_eq!(fits(Some(image), Some("contain")), fit, "{fit:?}");
            // The attribute alone reads the same keywords.
            let keyword = match fit {
                ContentFit::Fill => "fill",
                ContentFit::Contain => "contain",
                ContentFit::Cover => "cover",
                ContentFit::None => "none",
                ContentFit::ScaleDown => "scale-down",
            };
            assert_eq!(fits(None, Some(keyword)), fit, "{keyword}");
        }
        assert_eq!(fits(None, None), ContentFit::Fill);
        assert_eq!(fits(None, Some("bogus")), ContentFit::Fill);
    }

    /// One block-flow line of mixed atoms, for Gate G.
    struct AtomLine {
        context: AppContext,
        document: DocumentId,
        shaper: NanaTextEngineShaper,
        block: StableNodeId,
        /// 40x20, empty: no line box.
        short: StableNodeId,
        /// 40 wide, `tall` high, 6 below: no line box.
        tall: StableNodeId,
        /// A 20x30 image, 4 below.
        image: StableNodeId,
        /// 50x30, `overflow: hidden`, holding a label.
        clipped: StableNodeId,
        /// A 60x60 text node of its own.
        text: StableNodeId,
        /// A 30x20 text node of its own, that neither sets the line's baseline
        /// nor its depth.
        note: StableNodeId,
    }

    impl AtomLine {
        fn new(tall: f32, text: &str, note: &str, short_label: bool) -> Self {
            let document = DocumentId::new(1).unwrap();
            let (mut b, page) = Builder::page(document, 1, 800.0);
            let block = b.element(
                page,
                LayoutStyle {
                    display: Some(DisplaySpec::Block),
                    width: Some(LengthSpec::Px(600.0)),
                    ..LayoutStyle::default()
                },
            );
            let atom = |width: f32, height: f32| LayoutStyle {
                display: Some(DisplaySpec::InlineBlock),
                width: Some(LengthSpec::Px(width)),
                height: Some(LengthSpec::Px(height)),
                ..LayoutStyle::default()
            };
            let short = b.element(block, atom(40.0, 20.0));
            let tall = b.element(
                block,
                LayoutStyle {
                    margin_bottom: Some(LengthSpec::Px(6.0)),
                    ..atom(40.0, tall)
                },
            );
            let image = b.element(
                block,
                LayoutStyle {
                    display: Some(DisplaySpec::InlineBlock),
                    margin_bottom: Some(LengthSpec::Px(4.0)),
                    ..image_layout("pic", Some(20.0), Some(30.0))
                },
            );
            let clipped = b.element(
                block,
                LayoutStyle {
                    overflow_y: nana_ui_core::OverflowSpec::Hidden,
                    ..atom(50.0, 30.0)
                },
            );
            b.label(clipped, "Clipped");
            let text = b.label(block, text);
            b.queue.set_style(text, styled(atom(60.0, 60.0)));
            let note = b.label(block, note);
            b.queue.set_style(note, styled(atom(30.0, 20.0)));
            if short_label {
                short_label_into(&mut b.queue, document, short);
            }
            let mut context = AppContext::new();
            context.commit_mutations(b.queue).unwrap();
            let mut line = Self {
                context,
                document,
                shaper: bundled_face_shaper(),
                block,
                short,
                tall,
                image,
                clipped,
                text,
                note,
            };
            line.frame();
            line
        }

        fn frame(&mut self) -> nana_ui_core::WorkCounters {
            product_frame(
                &mut self.context,
                self.document,
                viewport(),
                &mut self.shaper,
            )
        }

        fn commit(&mut self, queue: MutationQueue) {
            self.context.commit_mutations(queue).unwrap();
            self.frame();
        }

        /// The line's baseline, as the text atom's last line puts it.
        fn text_baseline(&self) -> f32 {
            self.last_baseline(self.text)
        }

        /// Where text atom `id` puts its last line's baseline, kept inside its
        /// box.
        fn last_baseline(&self, id: StableNodeId) -> f32 {
            let world = self.context.world();
            let (_, layout) = world.text_layout(id).expect("shaped text");
            let last = layout.lines.last().expect("a line").metrics.baseline_y_px;
            let bounds = world.layout_box(id).unwrap();
            bounds.y + last.min(bounds.height)
        }

        /// Every atom on one baseline, and the line box around them. `short`
        /// sits by its bottom edge while it has no line box.
        fn check(&self, short_empty: bool) {
            let world = self.context.world();
            let bounds = |id| world.layout_box(id).unwrap();
            let baseline = self.text_baseline();
            let lines = world.text_layout(self.text).unwrap().1.lines.len();
            let mut on_edge = vec![(self.tall, 6.0), (self.image, 4.0), (self.clipped, 0.0)];
            if short_empty {
                on_edge.push((self.short, 0.0));
            }
            for (id, below) in on_edge {
                let edge = bounds(id).y + bounds(id).height + below;
                assert!(
                    (edge - baseline).abs() < 0.01,
                    "{id:?}: bottom margin edge {edge} vs the text's last baseline {baseline} \
                     ({lines} lines)"
                );
            }
            let note = self.last_baseline(self.note);
            assert!(
                (note - baseline).abs() < 0.01,
                "the note's last baseline {note} vs {baseline}"
            );
            // Not top-aligned: the atoms' tops differ.
            assert!(bounds(self.short).y > bounds(self.tall).y + 1.0);
            // The line box: highest top to lowest bottom, margins included.
            let block = bounds(self.block);
            let atoms = [
                (self.short, 0.0),
                (self.tall, 6.0),
                (self.image, 4.0),
                (self.clipped, 0.0),
                (self.text, 0.0),
                (self.note, 0.0),
            ];
            let top = atoms
                .iter()
                .map(|&(id, _)| bounds(id).y)
                .fold(f32::INFINITY, f32::min);
            let bottom = atoms
                .iter()
                .map(|&(id, below)| bounds(id).y + bounds(id).height + below)
                .fold(f32::NEG_INFINITY, f32::max);
            assert!(
                (top - block.y).abs() < 0.01,
                "line top {top} vs {}",
                block.y
            );
            assert!(
                (block.height - (bottom - top)).abs() < 0.01,
                "block {} vs line box {}",
                block.height,
                bottom - top
            );
        }
    }

    /// Gate G.
    #[test]
    fn issue202_g_atoms_on_a_block_line_share_one_baseline() {
        let mut line = AtomLine::new(50.0, "Word", "a", false);
        assert_eq!(
            line.context
                .world()
                .layout_node(line.text)
                .unwrap()
                .participation,
            Some(Part::AtomicInline)
        );
        line.check(true);
        let first = line.text_baseline();

        // A taller atom lowers the baseline.
        let mut queue = MutationQueue::new();
        queue.set_style(
            line.tall,
            styled(LayoutStyle {
                display: Some(DisplaySpec::InlineBlock),
                width: Some(LengthSpec::Px(40.0)),
                height: Some(LengthSpec::Px(70.0)),
                margin_bottom: Some(LengthSpec::Px(6.0)),
                ..LayoutStyle::default()
            }),
        );
        line.commit(queue);
        line.check(true);
        assert!(line.text_baseline() > first + 1.0);
        assert_matches_cold(
            &mut line.context,
            &mut AtomLine::new(70.0, "Word", "a", false).context,
            line.document,
        );

        // Text that wraps in a box of the same size moves its last baseline:
        // the box rises to keep that baseline on the line's.
        let before = line.context.world().layout_box(line.text).unwrap().y;
        let mut queue = MutationQueue::new();
        queue.set_text(
            line.text,
            crate::TextContent {
                value: "Word word word".into(),
            },
        );
        line.commit(queue);
        assert!(
            line.context
                .world()
                .text_layout(line.text)
                .unwrap()
                .1
                .lines
                .len()
                > 1
        );
        assert_eq!(
            line.context.world().layout_box(line.text).unwrap().height,
            60.0
        );
        line.check(true);
        assert!(line.context.world().layout_box(line.text).unwrap().y < before - 1.0);
        assert_matches_cold(
            &mut line.context,
            &mut AtomLine::new(70.0, "Word word word", "a", false).context,
            line.document,
        );

        // A baseline that moves in a box of the same size, under a line of the
        // same height: the line's box and size stay, the atom moves.
        let block = line.context.world().layout_box(line.block).unwrap();
        let note = line.context.world().layout_box(line.note).unwrap();
        let mut queue = MutationQueue::new();
        queue.set_text(
            line.note,
            crate::TextContent {
                value: "aa aa aa aa".into(),
            },
        );
        line.commit(queue);
        assert!(
            line.context
                .world()
                .text_layout(line.note)
                .unwrap()
                .1
                .lines
                .len()
                > 1
        );
        line.check(true);
        assert_eq!(line.context.world().layout_box(line.block).unwrap(), block);
        let moved = line.context.world().layout_box(line.note).unwrap();
        assert_eq!((moved.width, moved.height), (note.width, note.height));
        assert!(moved.y < note.y - 1.0, "{moved:?} vs {note:?}");
        assert_matches_cold(
            &mut line.context,
            &mut AtomLine::new(70.0, "Word word word", "aa aa aa aa", false).context,
            line.document,
        );

        // A line box arriving in an empty atom gives it a baseline of its own.
        let mut queue = MutationQueue::new();
        short_label_into(&mut queue, line.document, line.short);
        line.commit(queue);
        line.check(false);
        assert_matches_cold(
            &mut line.context,
            &mut AtomLine::new(70.0, "Word word word", "aa aa aa aa", true).context,
            line.document,
        );
    }

    /// A label for Gate G's empty atom, at an id past the page's.
    fn short_label_into(queue: &mut MutationQueue, document: DocumentId, short: StableNodeId) {
        let label = StableNodeId::new(1_000).unwrap();
        queue.create(label, document, crate::NodeKind::Text);
        queue.insert(short, label, None);
        queue.set_text(
            label,
            crate::TextContent {
                value: "Short".into(),
            },
        );
    }

    /// One block-flow line whose atoms hold their text in descendants, for
    /// Gate H.
    struct NestedLine {
        context: AppContext,
        document: DocumentId,
        shaper: NanaTextEngineShaper,
        block: StableNodeId,
        /// 40x20, empty: its bottom edge is the line's baseline.
        edge: StableNodeId,
        /// 120x80 with 7 of padding on top, holding a block `spacer` high and a
        /// label below it.
        holder: StableNodeId,
        spacer: StableNodeId,
        holder_label: StableNodeId,
        /// 60 wide, its height its content's: a block holding a label that wraps.
        nested: StableNodeId,
        nested_label: StableNodeId,
        /// 70 wide, its height its content's: two inline-block labels 60 wide,
        /// one line box each.
        stacked: StableNodeId,
        stacked_labels: [StableNodeId; 2],
        /// 80 wide, its height its content's: a fixed 70x50 block holding a
        /// label.
        boxed: StableNodeId,
        boxed_label: StableNodeId,
    }

    impl NestedLine {
        fn new(spacer: f32, holder_text: &str, nested_text: &str, boxed_text: &str) -> Self {
            let document = DocumentId::new(1).unwrap();
            let (mut b, page) = Builder::page(document, 1, 800.0);
            let block = b.element(
                page,
                LayoutStyle {
                    display: Some(DisplaySpec::Block),
                    width: Some(LengthSpec::Px(600.0)),
                    ..LayoutStyle::default()
                },
            );
            let inline_block = |width: f32, height: Option<f32>| LayoutStyle {
                display: Some(DisplaySpec::InlineBlock),
                width: Some(LengthSpec::Px(width)),
                height: height.map(LengthSpec::Px),
                ..LayoutStyle::default()
            };
            let edge = b.element(block, inline_block(40.0, Some(20.0)));
            let holder = b.element(
                block,
                LayoutStyle {
                    padding_top: Some(LengthSpec::Px(7.0)),
                    ..inline_block(120.0, Some(80.0))
                },
            );
            let spacer = b.element(holder, spacer_style(spacer));
            let holder_label = b.label(holder, holder_text);
            let nested = b.element(block, inline_block(60.0, None));
            let inner = b.element(
                nested,
                LayoutStyle {
                    display: Some(DisplaySpec::Block),
                    ..LayoutStyle::default()
                },
            );
            let nested_label = b.label(inner, nested_text);
            let stacked = b.element(block, inline_block(70.0, None));
            let stacked_labels = ["Up", "Down"].map(|text| {
                let label = b.label(stacked, text);
                b.queue
                    .set_style(label, styled(inline_block(60.0, Some(18.0))));
                label
            });
            let boxed = b.element(block, inline_block(80.0, None));
            let fixed_block = b.element(
                boxed,
                LayoutStyle {
                    display: Some(DisplaySpec::Block),
                    width: Some(LengthSpec::Px(70.0)),
                    height: Some(LengthSpec::Px(50.0)),
                    ..LayoutStyle::default()
                },
            );
            let boxed_label = b.label(fixed_block, boxed_text);
            let mut context = AppContext::new();
            context.commit_mutations(b.queue).unwrap();
            let mut line = Self {
                context,
                document,
                shaper: bundled_face_shaper(),
                block,
                edge,
                holder,
                spacer,
                holder_label,
                nested,
                nested_label,
                stacked,
                stacked_labels,
                boxed,
                boxed_label,
            };
            line.frame();
            line
        }

        fn frame(&mut self) -> nana_ui_core::WorkCounters {
            product_frame(
                &mut self.context,
                self.document,
                viewport(),
                &mut self.shaper,
            )
        }

        fn commit(&mut self, queue: MutationQueue) -> nana_ui_core::WorkCounters {
            self.context.commit_mutations(queue).unwrap();
            self.frame()
        }

        fn lines(&self, label: StableNodeId) -> usize {
            self.context
                .world()
                .text_layout(label)
                .expect("shaped text")
                .1
                .lines
                .len()
        }

        /// Where `label` puts the baseline of its line `index` (the last one
        /// when `None`), in the document.
        fn label_baseline(&self, label: StableNodeId, index: Option<usize>) -> f32 {
            let world = self.context.world();
            let (_, layout) = world.text_layout(label).expect("shaped text");
            let line = match index {
                Some(index) => &layout.lines[index],
                None => layout.lines.last().expect("a line"),
            };
            world.layout_box(label).unwrap().y + line.metrics.baseline_y_px
        }

        /// Each atom on the line's baseline by its descendant's last line.
        fn check(&self) {
            let world = self.context.world();
            let edge = world.layout_box(self.edge).unwrap();
            let baseline = edge.y + edge.height;
            for label in [
                self.holder_label,
                self.nested_label,
                self.stacked_labels[1],
                self.boxed_label,
            ] {
                let last = self.label_baseline(label, None);
                assert!(
                    (last - baseline).abs() < 0.01,
                    "{label:?}: last baseline {last} vs the line's {baseline} ({} lines)",
                    self.lines(label)
                );
            }
            // The stacked atom's second line, not its first.
            let [up, down] = self.stacked_labels.map(|id| world.layout_box(id).unwrap());
            assert!(down.y >= up.y + up.height - 0.01, "{up:?} {down:?}");
            // Not its own font's first line: the label sits below a spacer.
            let holder = world.layout_box(self.holder).unwrap();
            let spacer = world.layout_box(self.spacer).unwrap();
            assert!(spacer.height > 0.0 && spacer.y >= holder.y + 7.0 - 0.01);
            assert!(
                world.layout_box(self.holder_label).unwrap().y >= spacer.y + spacer.height - 0.01
            );
        }
    }

    fn spacer_style(height: f32) -> LayoutStyle {
        LayoutStyle {
            display: Some(DisplaySpec::Block),
            height: Some(LengthSpec::Px(height)),
            ..LayoutStyle::default()
        }
    }

    /// Gate H: an inline-block whose text lies in its descendants sits on the
    /// line by the baseline of its last line box, where that line box sits
    /// inside it (CSS 2.1 §10.8.1), not by its own font's first line. A
    /// descendant's change that moves that line places the line again; one
    /// that does not leaves every box where it was. Each matches a cold layout.
    #[test]
    fn issue202_h_an_inline_block_sits_by_its_descendants_last_line() {
        let mut line = NestedLine::new(10.0, "Nested", "aa aa aa aa aa", "Box");
        assert!(
            line.lines(line.nested_label) > 1,
            "the nested label wraps: {} lines",
            line.lines(line.nested_label)
        );
        assert_eq!(line.lines(line.holder_label), 1);
        line.check();
        // The second line, not the first, is on the line's baseline.
        let first = line.label_baseline(line.nested_label, Some(0));
        assert!(first < line.label_baseline(line.nested_label, None) - 1.0);

        // How far below the holder's top the line's baseline is.
        let depth = |line: &NestedLine| {
            let world = line.context.world();
            let edge = world.layout_box(line.edge).unwrap();
            edge.y + edge.height - world.layout_box(line.holder).unwrap().y
        };

        // Text that wraps in a box of the same size moves its last line down:
        // the line's baseline sits that much lower inside the holder.
        let holder = line.context.world().layout_box(line.holder).unwrap();
        let before = depth(&line);
        let mut queue = MutationQueue::new();
        queue.set_text(
            line.holder_label,
            crate::TextContent {
                value: "Nested nested nested nested".into(),
            },
        );
        line.commit(queue);
        assert!(line.lines(line.holder_label) > 1);
        line.check();
        let moved = line.context.world().layout_box(line.holder).unwrap();
        assert_eq!((moved.width, moved.height), (holder.width, holder.height));
        assert!(depth(&line) > before + 1.0, "{} vs {before}", depth(&line));
        assert_matches_cold(
            &mut line.context,
            &mut NestedLine::new(10.0, "Nested nested nested nested", "aa aa aa aa aa", "Box")
                .context,
            line.document,
        );

        // A taller box above the label moves it, and the line's baseline sits
        // as much lower inside the holder.
        let before = depth(&line);
        let mut queue = MutationQueue::new();
        queue.set_style(line.spacer, styled(spacer_style(20.0)));
        line.commit(queue);
        line.check();
        assert!(
            (depth(&line) - before - 10.0).abs() < 0.01,
            "{} vs {before}",
            depth(&line)
        );
        assert_matches_cold(
            &mut line.context,
            &mut NestedLine::new(20.0, "Nested nested nested nested", "aa aa aa aa aa", "Box")
                .context,
            line.document,
        );

        // Text that keeps its lines where they were moves nothing.
        let boxes = |line: &NestedLine| {
            [
                line.block,
                line.edge,
                line.holder,
                line.nested,
                line.stacked,
                line.boxed,
            ]
            .map(|id| line.context.world().layout_box(id).unwrap())
        };
        let before = boxes(&line);
        let lines = line.lines(line.nested_label);
        let mut queue = MutationQueue::new();
        queue.set_text(
            line.nested_label,
            crate::TextContent {
                value: "bb bb bb bb bb".into(),
            },
        );
        let work = line.commit(queue);
        assert_eq!(work.layout_full_document_fallbacks, 0);
        assert_eq!(line.lines(line.nested_label), lines);
        line.check();
        assert_eq!(boxes(&line), before);
        assert_matches_cold(
            &mut line.context,
            &mut NestedLine::new(20.0, "Nested nested nested nested", "bb bb bb bb bb", "Box")
                .context,
            line.document,
        );

        // Text wrapping inside a fixed block of the atom: the block keeps its
        // size, the last line box moves, and the line is placed again.
        let before = boxes(&line);
        let mut queue = MutationQueue::new();
        queue.set_text(
            line.boxed_label,
            crate::TextContent {
                value: "Box box box box".into(),
            },
        );
        line.commit(queue);
        assert!(line.lines(line.boxed_label) > 1);
        line.check();
        assert_ne!(boxes(&line), before);
        assert_matches_cold(
            &mut line.context,
            &mut NestedLine::new(
                20.0,
                "Nested nested nested nested",
                "bb bb bb bb bb",
                "Box box box box",
            )
            .context,
            line.document,
        );
    }

    /// Gate I: a line box of an ordinary block starts from the block's strut
    /// (CSS 2.1 §10.8.1), the ascent and descent of its own font at its line
    /// height: an empty inline-block, which sits by its bottom edge, leaves the
    /// font's descent below it, and a short one sits no higher than the strut's
    /// ascent. A block-level box holds a line alone, with no strut. A native
    /// paragraph's line has one too.
    #[test]
    fn issue202_i_a_line_box_starts_with_the_blocks_strut() {
        let document = DocumentId::new(1).unwrap();
        let (mut b, page) = Builder::page(document, 1, 800.0);
        let font = 20.0f32;
        let block = |line_height| LayoutStyle {
            display: Some(DisplaySpec::Block),
            width: Some(LengthSpec::Px(600.0)),
            font_size: Some(font),
            line_height,
            ..LayoutStyle::default()
        };
        let atom = |width: f32, height: f32| LayoutStyle {
            display: Some(DisplaySpec::InlineBlock),
            width: Some(LengthSpec::Px(width)),
            height: Some(LengthSpec::Px(height)),
            ..LayoutStyle::default()
        };
        // No leading: the strut is the em box, its descent the font's.
        let tight = b.element(
            page,
            block(Some(nana_ui_core::LineHeightSpec::Absolute(font))),
        );
        let tight_atom = b.element(tight, atom(40.0, 30.0));
        let tight_short = b.element(tight, atom(10.0, 6.0));
        // The default line height adds half its leading on each side.
        let normal = b.element(page, block(None));
        let normal_atom = b.element(normal, atom(40.0, 30.0));
        // A block-level box alone on its line.
        let blocky = b.element(page, block(None));
        b.element(
            blocky,
            LayoutStyle {
                display: Some(DisplaySpec::Block),
                ..atom(40.0, 30.0)
            },
        );
        let paragraph_atom = b.detached(atom(40.0, 30.0));
        let mut context = AppContext::new();
        context.commit_mutations(b.queue).unwrap();
        let paragraph = context
            .create_component(
                document,
                InlineFlow::new(InlineContent::builder().child(paragraph_atom).build())
                    .width(LengthSpec::Px(600.0)),
            )
            .unwrap()
            .stable_id();
        let mut shaper = bundled_face_shaper();
        product_frame(&mut context, document, viewport(), &mut shaper);
        let world = context.world();
        let bounds = |id| world.layout_box(id).unwrap();
        // The block's own face's ascent and descent (#382), the half-leading
        // what the line height leaves around them.
        let face = world
            .block_strut(tight)
            .expect("the face reports its metrics");
        let (ascent, descent) = (font * face.ascent, font * face.descent);
        let half_leading = |line_height: f32| (line_height - (ascent + descent)) / 2.0;

        // The tall atom sets the line's top; the strut its depth.
        let (line, tall, short) = (bounds(tight), bounds(tight_atom), bounds(tight_short));
        assert!((tall.y - line.y).abs() < 0.01, "{tall:?} vs {line:?}");
        let baseline = tall.y + tall.height;
        let gap = line.y + line.height - baseline;
        let below = descent + half_leading(font);
        assert!(
            (gap - below).abs() < 0.01,
            "the gap below an empty atom {gap} vs the strut's descent {below}"
        );
        assert!((short.y + short.height - baseline).abs() < 0.01);

        // Half the leading of the default line height on each side.
        let (line, tall) = (bounds(normal), bounds(normal_atom));
        let gap = line.y + line.height - (tall.y + tall.height);
        let below = descent + half_leading(font * 1.2);
        assert!((gap - below).abs() < 0.01, "{gap} vs {below}");

        // A line of a block-level box alone has no strut.
        assert_eq!(bounds(blocky).height, 30.0);

        // A native paragraph's line leaves room below its atom as well; its
        // strut is its face's, shaped by nana-text.
        let (line, held) = (bounds(paragraph), bounds(paragraph_atom));
        assert!(
            line.y + line.height > held.y + held.height + 1.0,
            "{held:?} in {line:?}"
        );

        // A short atom sits no higher than the strut's ascent: the line's top
        // is the strut's.
        let mut queue = MutationQueue::new();
        queue.set_style(tight_atom, styled(atom(40.0, 4.0)));
        context.commit_mutations(queue).unwrap();
        product_frame(&mut context, document, viewport(), &mut shaper);
        let world = context.world();
        let (line, tall) = (
            world.layout_box(tight).unwrap(),
            world.layout_box(tight_atom).unwrap(),
        );
        assert!(
            (tall.y + tall.height - (line.y + ascent + half_leading(font))).abs() < 0.01,
            "{tall:?} in {line:?}"
        );
        assert!((line.height - font).abs() < 0.01, "{line:?}");
    }
}

mod overlay_layers {
    //! Issue #204: an overlay stack is its own formatting context, not a flex row.
    //!
    //! `Stack::layers()` (`display: layers`) gives every in-flow child the same
    //! area, as SwiftUI `ZStack`, Compose `Box` and Flutter `Stack` do: each child
    //! aligns in it, the container takes the largest child's size, later children
    //! sit above earlier ones, and an out-of-flow child keeps its own placement.

    use std::sync::Arc;

    use nana_ui_core::{AlignSpec, DirSpec, JustifySpec, LayoutStyle, LengthSpec};
    use nana_ui_core::{PositionSpec, WorkCounters};

    use crate::world::reflow_oracle::{
        Builder, assert_matches_cold, bundled_face_shaper, fixed, product_frame, styled,
    };
    use crate::world::{DocumentId, StableNodeId};
    use crate::{
        AppContext, Button, FormattingContextKind, LayoutViewport, MutationQueue,
        NanaTextEngineShaper, ParticipationKind, Stack,
    };

    fn viewport() -> LayoutViewport {
        LayoutViewport::new(800.0, 600.0)
    }

    fn layers(stack: Stack) -> LayoutStyle {
        (*stack.style_ref().layout).clone()
    }

    fn sized(width: f32, height: f32) -> LayoutStyle {
        LayoutStyle {
            position: PositionSpec::Static,
            ..fixed(width, height)
        }
    }

    struct Page {
        context: AppContext,
        document: DocumentId,
        shaper: NanaTextEngineShaper,
        stack: StableNodeId,
        children: Vec<StableNodeId>,
    }

    impl Page {
        /// `stack` holding one child per entry of `children`, under a page.
        fn new(stack: LayoutStyle, children: &[LayoutStyle]) -> Self {
            let document = DocumentId::new(1).unwrap();
            let (mut b, page) = Builder::page(document, 1, 800.0);
            let id = b.element(page, stack);
            let children = children
                .iter()
                .map(|child| b.element(id, child.clone()))
                .collect();
            let mut context = AppContext::new();
            context.commit_mutations(b.queue).unwrap();
            let mut page = Self {
                context,
                document,
                shaper: bundled_face_shaper(),
                stack: id,
                children,
            };
            page.frame();
            page
        }

        fn frame(&mut self) -> WorkCounters {
            product_frame(
                &mut self.context,
                self.document,
                viewport(),
                &mut self.shaper,
            )
        }

        fn rect(&self, id: StableNodeId) -> (f32, f32, f32, f32) {
            let b = self.context.world().layout_box(id).unwrap();
            (b.x, b.y, b.width, b.height)
        }

        /// `id`'s offset from the stack's content origin.
        fn offset(&self, id: StableNodeId) -> (f32, f32) {
            let (sx, sy, ..) = self.rect(self.stack);
            let (x, y, ..) = self.rect(id);
            (x - sx, y - sy)
        }
    }

    #[test]
    fn issue204_layers_share_one_area_and_size_to_the_largest() {
        let absolute = LayoutStyle {
            position: PositionSpec::Absolute,
            offset_left: Some(LengthSpec::Px(300.0)),
            offset_top: Some(LengthSpec::Px(0.0)),
            ..sized(500.0, 500.0)
        };
        let mut page = Page::new(
            layers(Stack::layers()),
            &[sized(100.0, 40.0), sized(60.0, 80.0), absolute],
        );
        let world = page.context.world();
        let node = world.layout_node(page.stack).unwrap();
        assert_eq!(node.established, Some(FormattingContextKind::Layers));
        let parts: Vec<_> = page
            .children
            .iter()
            .map(|id| world.layout_node(*id).unwrap().participation)
            .collect();
        assert_eq!(
            parts,
            [
                Some(ParticipationKind::LayerItem),
                Some(ParticipationKind::LayerItem),
                Some(ParticipationKind::Absolute),
            ]
        );
        // As large as the largest layer; the positioned child does not count.
        let (.., width, height) = page.rect(page.stack);
        assert_eq!((width, height), (100.0, 80.0));
        // Both layers start at the top left and keep their own sizes.
        assert_eq!(page.offset(page.children[0]), (0.0, 0.0));
        assert_eq!(page.offset(page.children[1]), (0.0, 0.0));
        assert_eq!(page.rect(page.children[0]).2, 100.0);
        assert_eq!(page.rect(page.children[1]).3, 80.0);
        assert_eq!(page.offset(page.children[2]), (300.0, 0.0));

        let mut cold = Page::new(
            layers(Stack::layers()),
            &[
                sized(100.0, 40.0),
                sized(60.0, 80.0),
                LayoutStyle {
                    position: PositionSpec::Absolute,
                    offset_left: Some(LengthSpec::Px(300.0)),
                    offset_top: Some(LengthSpec::Px(0.0)),
                    ..sized(500.0, 500.0)
                },
            ],
        );
        // A layer growing moves nothing else and still matches a cold layout.
        let mut queue = MutationQueue::new();
        queue.set_style(page.children[1], styled(sized(160.0, 80.0)));
        page.context.commit_mutations(queue).unwrap();
        page.frame();
        assert_eq!(page.rect(page.stack).2, 160.0);
        assert_eq!(page.offset(page.children[0]), (0.0, 0.0));
        let mut queue = MutationQueue::new();
        queue.set_style(cold.children[1], styled(sized(160.0, 80.0)));
        cold.context.commit_mutations(queue).unwrap();
        cold.frame();
        assert_matches_cold(&mut page.context, &mut cold.context, page.document);
    }

    #[test]
    fn issue204_layers_align_each_child_in_the_shared_area() {
        let anchors = [AlignSpec::Start, AlignSpec::Center, AlignSpec::End];
        let justify = [JustifySpec::Start, JustifySpec::Center, JustifySpec::End];
        for (row, align) in anchors.iter().enumerate() {
            for (column, justify) in justify.iter().enumerate() {
                // The anchoring an application builds from the facade: a layer
                // covering a positioned 300x200 box, one 40x20 part at an anchor.
                let mut stack = layers(Stack::overlay_layer().align(*align).justify(*justify));
                stack.overflow_x = nana_ui_core::OverflowSpec::Visible;
                stack.overflow_y = nana_ui_core::OverflowSpec::Visible;
                let document = DocumentId::new(1).unwrap();
                let (mut b, page) = Builder::page(document, 1, 800.0);
                let frame = b.element(
                    page,
                    LayoutStyle {
                        position: PositionSpec::Relative,
                        ..sized(300.0, 200.0)
                    },
                );
                let layer = b.element(frame, stack);
                let part = b.element(layer, sized(40.0, 20.0));
                let mut context = AppContext::new();
                context.commit_mutations(b.queue).unwrap();
                product_frame(
                    &mut context,
                    document,
                    viewport(),
                    &mut bundled_face_shaper(),
                );
                let world = context.world();
                let frame = world.layout_box(frame).unwrap();
                assert_eq!(world.layout_box(layer).unwrap(), frame);
                let part = world.layout_box(part).unwrap();
                let expected = (
                    column as f32 * (300.0 - 40.0) / 2.0,
                    row as f32 * (200.0 - 20.0) / 2.0,
                );
                assert_eq!(
                    (part.x - frame.x, part.y - frame.y),
                    expected,
                    "{align:?} {justify:?}"
                );
            }
        }

        // A child's own `align-self` / `justify-self` wins, `stretch` fills the
        // area, and `start` follows the writing direction.
        let mut stack = layers(Stack::layers().width(LengthSpec::Px(300.0)));
        stack.height = Some(LengthSpec::Px(200.0));
        stack.dir = Some(DirSpec::Rtl);
        let corner = LayoutStyle {
            align_self: Some(AlignSpec::End),
            justify_self: Some(AlignSpec::End),
            ..sized(40.0, 20.0)
        };
        let fill = LayoutStyle {
            align_self: Some(AlignSpec::Stretch),
            justify_self: Some(AlignSpec::Stretch),
            ..LayoutStyle::default()
        };
        let page = Page::new(stack, &[sized(40.0, 20.0), corner, fill]);
        assert_eq!(page.offset(page.children[0]), (260.0, 0.0));
        assert_eq!(page.offset(page.children[1]), (0.0, 180.0));
        assert_eq!(page.rect(page.children[2]).2, 300.0);
        assert_eq!(page.rect(page.children[2]).3, 200.0);
    }

    #[test]
    fn issue204_the_later_layer_takes_the_pointer() {
        let document = DocumentId::new(1).unwrap();
        let mut cx = AppContext::new();
        let stack = cx.create_component(document, Stack::layers()).unwrap();
        let below = cx
            .create_detached_component(
                document,
                Button::new("Below").layout(Arc::new(sized(120.0, 40.0))),
            )
            .unwrap();
        let above = cx
            .create_detached_component(
                document,
                Button::new("Above").layout(Arc::new(sized(60.0, 40.0))),
            )
            .unwrap();
        let mut queue = MutationQueue::new();
        queue.insert(stack.stable_id(), below.stable_id(), None);
        queue.insert(stack.stable_id(), above.stable_id(), None);
        cx.commit_mutations(queue).unwrap();
        product_frame(&mut cx, document, viewport(), &mut bundled_face_shaper());
        cx.rebuild_hit_test(document);
        let world = cx.world();
        let origin = world.layout_box(stack.stable_id()).unwrap();
        // Where the two overlap the later one is on top; past it, the earlier.
        assert_eq!(
            world.hit_test(document, origin.x + 30.0, origin.y + 20.0),
            Some(above.stable_id())
        );
        assert_eq!(
            world.hit_test(document, origin.x + 90.0, origin.y + 20.0),
            Some(below.stable_id())
        );
    }

    /// A facade is only spelling. The preset constructor, the view function and
    /// `Stack::from_layout` with the preset's intent mount the same component
    /// over the same nodes, with the same intent. Their first frame, a local edit
    /// and an idle frame cost the same layout work. (CSS lowers at build time to
    /// `Stack::from_layout` plus style data, so it takes the third path; its
    /// structural parity is in `tests/view_macro.rs`.)
    #[test]
    fn issue204_facades_share_identity_and_layout_work() {
        use crate::view::{IntoView, button, row, widget};
        use nana_ui_core::{AlignSpec, DisplaySpec, FlexDirection};

        struct Run {
            nodes: usize,
            kinds: Vec<crate::NodeKind>,
            intent: Arc<LayoutStyle>,
            work: [(usize, usize, usize, usize); 3],
        }
        let document = DocumentId::new(1).unwrap();
        let preset_intent = LayoutStyle {
            display: Some(DisplaySpec::Flex),
            direction: Some(FlexDirection::Row),
            gap: Some(LengthSpec::Px(0.0)),
            align_items: AlignSpec::Center,
            width: Some(LengthSpec::Shrink),
            ..LayoutStyle::default()
        };
        let cost = |work: WorkCounters| {
            (
                work.entities_spawned,
                work.layout_measure_nodes,
                work.layout_placement_nodes,
                work.layout_context_transitions,
            )
        };
        let runs: Vec<Run> = (0..3)
            .map(|facade| {
                let mut cx = AppContext::new();
                let mut shaper = bundled_face_shaper();
                let intent = preset_intent.clone();
                let _view = cx
                    .mount_view_root(document, move || {
                        let buttons = (0..50).map(|_| button("Save")).collect::<Vec<_>>();
                        match facade {
                            0 => widget(Stack::row(0.0)).children(buttons).into_any(),
                            1 => row().children(buttons).into_any(),
                            _ => widget(Stack::from_layout(intent))
                                .children(buttons)
                                .into_any(),
                        }
                    })
                    .unwrap();
                let first = product_frame(&mut cx, document, viewport(), &mut shaper);
                let order = cx.world().document_order(document);
                let root = order[0];
                let child = cx.world().child_ids(root)[7];
                let mut queue = MutationQueue::new();
                queue.set_style(child, styled(sized(90.0, 30.0)));
                cx.commit_mutations(queue).unwrap();
                let edit = product_frame(&mut cx, document, viewport(), &mut shaper);
                // The next frame is idle: nothing is left to do, and its own
                // counters (not the last non-empty snapshot) are all zero.
                product_frame(&mut cx, document, viewport(), &mut shaper);
                assert!(cx.take_system_work().is_empty(), "facade {facade}");
                let idle = cx.world().counters.frame_counters;
                let world = cx.world();
                Run {
                    nodes: world.len(),
                    kinds: order
                        .iter()
                        .map(|id| world.node(*id).unwrap().kind.clone())
                        .collect(),
                    intent: world.layout_node(root).unwrap().intent,
                    work: [cost(first), cost(edit), cost(idle)],
                }
            })
            .collect();
        let preset = &runs[0];
        assert!(preset.work[0].1 > 0 && preset.work[1].1 > 0);
        assert_eq!(preset.work[2], (0, 0, 0, 0), "an idle frame does nothing");
        for (facade, run) in runs.iter().enumerate().skip(1) {
            assert_eq!(run.nodes, preset.nodes, "facade {facade}");
            assert_eq!(run.kinds, preset.kinds, "facade {facade}");
            assert!(
                preset.intent.changed_fields(&run.intent).is_empty(),
                "facade {facade}: {:?}",
                preset.intent.changed_fields(&run.intent)
            );
            assert_eq!(run.work, preset.work, "facade {facade}");
        }
    }
}

mod scroll_counters {
    //! Issue #381: what the scroll path's work counters count, on a scroll-only
    //! frame, a viewport resize and a virtual list scroll.
    //!
    //! A page as tall as the viewport scrolls two lists, `list` and `other`.
    //! Each list holds a sticky header with two children, then [`ROWS`] rows.
    //!
    //! - `scroll_viewport_updates`: viewport size changes of a scroll container.
    //!   A store of equal metrics, or one that moved only the content, is not one.
    //! - `scroll_content_extent_recomputes`: content index entries recomputed.
    //!   Pure scrolling recomputes none.
    //! - `scroll_clip_updates`: hit entries, clips with them, the scroll path
    //!   rewrote. A list's sticky header is one: its children are held in its
    //!   own coordinates and keep their entries; its rows share the list's
    //!   shift and the other list is not reached.
    //! - Pure scrolling lays nothing out: measure, placement and shaping stay 0.
    //! - A virtual list maps onto `virtual_rows_materialized_from_layout`,
    //!   `entities_spawned` and `entities_despawned`.

    use nana_ui_core::WorkCounters;
    use nana_ui_core::{FlexDirection, LayoutStyle, LengthSpec, OverflowSpec, PositionSpec};

    use crate::world::reflow_oracle::{
        Builder, bundled_face_shaper, fixed, product_frame, resize_frame,
    };
    use crate::world::tests::incremental_work::virtual_scroll::{Feed, Pass, ROW};
    use crate::{
        AppContext, DocumentId, LayoutViewport, MutationQueue, NanaTextEngineShaper, ScrollOffset,
        StableNodeId,
    };

    /// Rows under each list's header.
    const ROWS: usize = 50;
    /// A list's scroll rewrites its sticky header alone: the header's two
    /// children are held relative to it, so they keep their entries.
    const STICKY_ENTRY: usize = 1;

    fn document() -> DocumentId {
        DocumentId::new(1).unwrap()
    }

    fn scroller(width: LengthSpec, height: LengthSpec) -> LayoutStyle {
        LayoutStyle {
            overflow_x: OverflowSpec::Auto,
            overflow_y: OverflowSpec::Auto,
            direction: Some(FlexDirection::Column),
            flex_shrink: Some(0.0),
            width: Some(width),
            height: Some(height),
            ..LayoutStyle::default()
        }
    }

    struct Page {
        cx: AppContext,
        shaper: NanaTextEngineShaper,
        viewport: LayoutViewport,
        page: StableNodeId,
        list: StableNodeId,
        other: StableNodeId,
    }

    impl Page {
        fn new() -> Self {
            let (mut b, root) = Builder::new(document(), 1);
            let page = b.element(
                root,
                scroller(LengthSpec::Percent(100.0), LengthSpec::Percent(100.0)),
            );
            let mut list = || {
                let list = b.element(page, scroller(LengthSpec::Px(300.0), LengthSpec::Px(200.0)));
                let header = b.element(
                    list,
                    LayoutStyle {
                        position: PositionSpec::Sticky,
                        offset_top: Some(LengthSpec::Px(0.0)),
                        flex_shrink: Some(0.0),
                        ..fixed(300.0, 20.0)
                    },
                );
                b.element(header, fixed(100.0, 10.0));
                b.element(header, fixed(100.0, 10.0));
                for _ in 0..ROWS {
                    b.element(
                        list,
                        LayoutStyle {
                            flex_shrink: Some(0.0),
                            ..fixed(300.0, 24.0)
                        },
                    );
                }
                list
            };
            let (list, other) = (list(), list());
            // Taller than the window, so the page scrolls too.
            b.element(
                page,
                LayoutStyle {
                    flex_shrink: Some(0.0),
                    ..fixed(300.0, 800.0)
                },
            );
            let mut cx = AppContext::new();
            cx.commit_mutations(b.queue).unwrap();
            let mut page = Self {
                cx,
                shaper: bundled_face_shaper(),
                viewport: LayoutViewport::new(800.0, 600.0),
                page,
                list,
                other,
            };
            page.frame();
            page.cx.rebuild_hit_test(document());
            page
        }

        /// A frame as `RuntimeDocument` runs a scroll-only one: lay out, then
        /// patch the hit index in place.
        fn frame(&mut self) -> WorkCounters {
            product_frame(&mut self.cx, document(), self.viewport, &mut self.shaper);
            for (scroller, delta) in self.cx.take_scroll_hit_updates() {
                self.cx.update_hit_test_scroll(document(), scroller, delta);
            }
            self.cx.last_work_counters()
        }

        fn scroll(&mut self, to: &[(StableNodeId, f32)]) -> WorkCounters {
            let mut q = MutationQueue::new();
            for &(id, y) in to {
                q.set_scroll_offset(id, ScrollOffset { x: 0.0, y });
            }
            self.cx.commit_mutations(q).unwrap();
            self.frame()
        }

        fn resize(&mut self, width: f32, height: f32) -> WorkCounters {
            self.viewport = LayoutViewport::new(width, height);
            resize_frame(&mut self.cx, document(), self.viewport, &mut self.shaper)
        }

        fn viewport_height(&self, id: StableNodeId) -> f32 {
            self.cx.world().scroll_metrics(id).unwrap().viewport_height
        }
    }

    /// `(scroll_viewport_updates, scroll_content_extent_recomputes,
    /// scroll_clip_updates)`.
    fn scroll_counters(work: &WorkCounters) -> (usize, usize, usize) {
        (
            work.scroll_viewport_updates,
            work.scroll_content_extent_recomputes,
            work.scroll_clip_updates,
        )
    }

    fn lays_nothing_out(work: &WorkCounters) -> bool {
        (
            work.layout_measure_nodes,
            work.layout_placement_nodes,
            work.text_shaped,
        ) == (0, 0, 0)
    }

    /// A scroll-only frame stores no viewport, recomputes no extent, and
    /// rewrites the entry of the sticky header it moves: one entry per list
    /// scrolled, not the list's rows and not the other list's header.
    #[test]
    fn issue381_a_scroll_only_frame_rewrites_the_sticky_entry_it_moves() {
        let mut page = Page::new();
        let (list, other, outer) = (page.list, page.other, page.page);
        let generation = page.cx.world().layout_generation();
        // One list: its sticky header.
        let work = page.scroll(&[(list, 100.0)]);
        assert!(lays_nothing_out(&work), "{work:?}");
        assert_eq!(scroll_counters(&work), (0, 0, STICKY_ENTRY));
        // The other list: its own header only.
        let work = page.scroll(&[(other, 40.0)]);
        assert!(lays_nothing_out(&work), "{work:?}");
        assert_eq!(scroll_counters(&work), (0, 0, STICKY_ENTRY));
        // Both: a header each.
        let work = page.scroll(&[(list, 20.0), (other, 300.0)]);
        assert!(lays_nothing_out(&work), "{work:?}");
        assert_eq!(scroll_counters(&work), (0, 0, 2 * STICKY_ENTRY));
        // The page: no sticky box hangs from it, so it moves every entry under
        // it by one shift and rewrites none.
        let work = page.scroll(&[(outer, 120.0)]);
        assert_eq!(page.cx.world().scroll_offset(outer).unwrap().y, 120.0);
        assert!(lays_nothing_out(&work), "{work:?}");
        assert_eq!(scroll_counters(&work), (0, 0, 0));
        assert_eq!(page.cx.world().layout_generation(), generation);
    }

    /// A viewport resize counts the one scroll container whose viewport moved.
    /// A store of equal metrics, and content that grows inside a viewport that
    /// holds, store no viewport.
    #[test]
    fn issue381_a_viewport_resize_counts_the_viewports_it_moved() {
        let mut page = Page::new();
        let (list, other, outer) = (page.list, page.other, page.page);
        assert_eq!(page.viewport_height(outer), 600.0);
        // The page follows the window; the lists keep their fixed size.
        let work = page.resize(800.0, 500.0);
        assert_eq!(page.viewport_height(outer), 500.0);
        assert_eq!(page.viewport_height(list), 200.0);
        assert_eq!(page.viewport_height(other), 200.0);
        assert_eq!(work.scroll_viewport_updates, 1, "{work:?}");
        // Only the page's own entry: no box under it moved.
        assert_eq!(work.scroll_content_extent_recomputes, 1, "{work:?}");
        assert_eq!(work.scroll_clip_updates, 0, "{work:?}");
        // The same metrics stored again: no work, so it is read as the
        // counters it leaves pending.
        let before = page.cx.last_work_counters();
        let metrics = page.cx.world().scroll_metrics(list);
        let mut q = MutationQueue::new();
        q.set_scroll_metrics(list, metrics);
        page.cx.commit_mutations(q).unwrap();
        assert_eq!(
            scroll_counters(&page.cx.last_work_counters()),
            scroll_counters(&before)
        );
        // A row grows inside `list`: its extent moves, its viewport holds.
        let row = page.cx.world().node(list).unwrap().children[1];
        let mut q = MutationQueue::new();
        q.set_style(
            row,
            crate::world::reflow_oracle::styled(LayoutStyle {
                flex_shrink: Some(0.0),
                ..fixed(300.0, 64.0)
            }),
        );
        page.cx.commit_mutations(q).unwrap();
        let before = page.cx.world().scroll_metrics(list).unwrap();
        let work = page.frame();
        let after = page.cx.world().scroll_metrics(list).unwrap();
        assert_eq!(after.content_height, before.content_height + 40.0);
        assert_eq!(work.scroll_viewport_updates, 0, "{work:?}");
        assert!(work.scroll_content_extent_recomputes > 0, "{work:?}");
    }

    /// A far scroll of a virtual list over `rows` rows, from a window at the
    /// top: the scroll-only frame, the list's sync, the frame that lays the new
    /// window out, and the rows the old and the new window held.
    fn far_scroll(rows: usize) -> (WorkCounters, Pass, WorkCounters, usize, usize) {
        let mut feed = Feed::new(rows);
        feed.cx.rebuild_hit_test(document());
        let unmounted = feed.mounted().len();
        // Far past the window: none of its rows stays mounted.
        feed.cx
            .scroll_to(
                feed.scroll,
                ScrollOffset {
                    x: 0.0,
                    y: 1_000.0 * ROW,
                },
            )
            .unwrap();
        feed.frame();
        for (scroller, delta) in feed.cx.take_scroll_hit_updates() {
            feed.cx.update_hit_test_scroll(document(), scroller, delta);
        }
        let scrolled = feed.cx.last_work_counters();
        let pass = feed.sync();
        let placed = feed.frame();
        (scrolled, pass, placed, unmounted, feed.mounted().len())
    }

    /// A virtual list scroll: the scroll-only frame stores and recomputes
    /// nothing and rewrites no clip; the list's sync materializes the new
    /// window and the frame after it spawns it and despawns the old one; that
    /// frame recomputes extents for the window it lays out, the same over a
    /// collection a hundred times larger.
    #[test]
    fn issue381_a_virtual_list_scroll_counts_the_window_it_mounts() {
        let (scrolled, pass, placed, unmounted, mounted) = far_scroll(10_000);
        assert!(lays_nothing_out(&scrolled), "{scrolled:?}");
        assert_eq!(scroll_counters(&scrolled), (0, 0, 0));
        // Materialized rows are the new window's; each row is the same number
        // of entities, spawned for the new window and despawned for the old.
        assert_eq!(pass.materialized, mounted, "{pass:?}");
        assert_eq!(placed.virtual_rows_materialized_from_layout, mounted);
        let per_row = placed.entities_spawned / mounted;
        assert!(per_row > 0, "{placed:?}");
        assert_eq!(placed.entities_spawned, per_row * mounted, "{placed:?}");
        assert_eq!(placed.entities_despawned, per_row * unmounted, "{placed:?}");
        assert_eq!(placed.scroll_viewport_updates, 0, "{placed:?}");
        // Each publish batch recomputes an entry at most once, and the world
        // holds only the window.
        let recomputes = placed.scroll_content_extent_recomputes;
        assert!(recomputes > 0, "{placed:?}");
        assert!(
            recomputes <= placed.layout_delta_commits * placed.entities_total,
            "{placed:?}"
        );
        let (_, _, larger, ..) = far_scroll(1_000_000);
        assert_eq!(scroll_counters(&larger), scroll_counters(&placed));
    }
}

mod aspect_ratio_sizing {
    //! Issue #382: the precision left after the baseline policy of #202.
    //!
    //! - Gate A: a box with a declared `aspect-ratio` and no content of its own
    //!   takes its height from the width min/max and both insets leave it (CSS
    //!   Sizing 4 §5), the same as a flex item, a grid item, an atomic inline
    //!   box and an absolutely positioned box. The ratio relates the boxes
    //!   `box-sizing` names, and the height it gives meets the box's own
    //!   `min-height` / `max-height`. An in-flow container's auto height reads
    //!   that height too (#393). Each matches a cold layout.
    //! - Gate B: an ordinary block's line boxes start from its own face's strut,
    //!   the ascent and descent `nana-text` reads for a native paragraph in the
    //!   same face, so the two leave the same room below an empty atom. A new
    //!   family, weight or font set moves the strut and the lines with it; a
    //!   language, a font size and changes beside the block do not lay it out
    //!   for its strut; an atom turned block-level keeps the strut while one
    //!   inline-level child remains and drops it after. Each step matches a cold
    //!   layout, and every pass checks every retained strut against a fresh
    //!   read (`UiWorld::audit_block_struts`).
    //! - Gate C: a display change republishes how the box takes part in its
    //!   parent's context, though its box stays put; children turned
    //!   inline-level again leave the flow plan their block recorded while they
    //!   were block-level, and lie on a line again.
    //! - Gate D: an item a flex row aligns by its first baseline, whose line
    //!   boxes are its descendants', sits by its first in-flow line box: below
    //!   a spacer in a fixed box, two blocks down, the first of two lines. A new
    //!   font size and a taller spacer inside the fixed box align the row again;
    //!   text that leaves the first line where it was, and text outside the row,
    //!   move nothing. Each step matches a cold layout.
    //! - Gate E: a block flow not packed to its start (`justify-content`
    //!   center, end, space-between, space-evenly; a reversed flow) sits by
    //!   where its first and last line boxes are, on a flex row and on a line.
    //!   A new justification aligns them again; each step matches a cold layout.
    //! - Gate F: a box whose lines run beside a float sits by where those lines
    //!   are, packed in the room the float leaves them: its first line below
    //!   the float on a flex row, its last wrapped below it on a line.
    //! - Gate G: a block flow wrapping into columns sits by its first and last
    //!   line boxes where those columns put them, packed to their end or
    //!   reversed; a taller flow in one column aligns them again.

    use nana_ui_core::{
        BoxSizing, DisplaySpec, FlexDirection, GridTrack, JustifySpec, LayoutStyle, LengthSpec,
        PositionSpec,
    };

    use std::sync::{Arc, Mutex};

    use nana_text::NativeTextEngine;
    use nana_text::font::{FaceDescriptor, FallbackPolicy, FontSystem, GenericFamily};

    use crate::world::NodeKind;
    use crate::world::reflow_oracle::{
        Builder, assert_matches_cold, bundled_face_shaper, measured, product_frame,
        product_frame_seeded, styled,
    };
    use crate::{
        AppContext, ComputedStyle, DocumentId, FontStrut, InlineContent, InlineFlow,
        LayoutViewport, MutationQueue, NanaTextEngineShaper, StableNodeId, TextShaper,
    };

    fn viewport() -> LayoutViewport {
        LayoutViewport::new(800.0, 600.0)
    }

    #[derive(Debug, Clone, Copy)]
    enum Context {
        Flex,
        Grid,
        Inline,
        Absolute,
        /// Absolute, its width from both horizontal insets.
        AbsoluteInsets,
    }

    impl Context {
        fn container(self) -> LayoutStyle {
            let mut layout = LayoutStyle {
                width: Some(LengthSpec::Px(600.0)),
                ..LayoutStyle::default()
            };
            match self {
                Self::Flex => {
                    layout.display = Some(DisplaySpec::Flex);
                    layout.direction = Some(FlexDirection::Row);
                }
                Self::Grid => {
                    layout.display = Some(DisplaySpec::Grid);
                    layout.grid_columns = Some(vec![GridTrack::Auto]);
                }
                Self::Inline => layout.display = Some(DisplaySpec::Block),
                Self::Absolute | Self::AbsoluteInsets => {
                    layout.display = Some(DisplaySpec::Block);
                    layout.position = PositionSpec::Relative;
                    layout.height = Some(LengthSpec::Px(300.0));
                }
            }
            layout
        }

        fn item(self, mut layout: LayoutStyle) -> LayoutStyle {
            match self {
                Self::Flex | Self::Grid => {}
                Self::Inline => layout.display = Some(DisplaySpec::InlineBlock),
                Self::Absolute => {
                    layout.position = PositionSpec::Absolute;
                    layout.offset_top = Some(LengthSpec::Px(0.0));
                    layout.offset_left = Some(LengthSpec::Px(0.0));
                }
                Self::AbsoluteInsets => {
                    layout.position = PositionSpec::Absolute;
                    layout.offset_top = Some(LengthSpec::Px(0.0));
                    layout.offset_left = Some(LengthSpec::Px(0.0));
                    // 600 wide: the insets leave the declared width, which a
                    // content box declares inside its padding.
                    let px = |spec: Option<LengthSpec>| {
                        spec.map_or(0.0, |spec| match spec {
                            LengthSpec::Px(px) => px,
                            other => panic!("{other:?}"),
                        })
                    };
                    let mut width = px(layout.width.take());
                    if layout.box_sizing == BoxSizing::ContentBox {
                        width += 2.0 * px(layout.padding);
                    }
                    layout.offset_right = Some(LengthSpec::Px(600.0 - width));
                }
            }
            layout
        }
    }

    fn ratio_box(edit: fn(&mut LayoutStyle)) -> LayoutStyle {
        let mut layout = LayoutStyle {
            width: Some(LengthSpec::Px(120.0)),
            aspect_ratio: Some(1.5),
            ..LayoutStyle::default()
        };
        edit(&mut layout);
        layout
    }

    /// One box in `context`, laid out once. Returns the box.
    fn page(context: Context, layout: LayoutStyle) -> (AppContext, DocumentId, StableNodeId) {
        let document = DocumentId::new(1).unwrap();
        let (mut b, page) = Builder::page(document, 1, 800.0);
        let container = b.element(page, context.container());
        let id = b.element(container, context.item(layout));
        let mut app = AppContext::new();
        app.commit_mutations(b.queue).unwrap();
        product_frame(&mut app, document, viewport(), &mut bundled_face_shaper());
        (app, document, id)
    }

    /// Gate A.
    #[test]
    fn issue382_a_declared_ratio_follows_the_clamped_width_everywhere() {
        type Edit = fn(&mut LayoutStyle);
        let cases: [(&str, Edit, (f32, f32)); 7] = [
            ("width", |_| {}, (120.0, 80.0)),
            (
                "max-width",
                |l| l.max_width = Some(LengthSpec::Px(60.0)),
                (60.0, 40.0),
            ),
            (
                "min-width",
                |l| l.min_width = Some(LengthSpec::Px(240.0)),
                (240.0, 160.0),
            ),
            // Border boxes: 60 wide, 40 tall, the padding inside both.
            (
                "border-box padding",
                |l| {
                    l.max_width = Some(LengthSpec::Px(60.0));
                    l.padding = Some(LengthSpec::Px(6.0));
                },
                (60.0, 40.0),
            ),
            // Content boxes: 120×80 inside, the padding outside both.
            (
                "content-box padding",
                |l| {
                    l.box_sizing = BoxSizing::ContentBox;
                    l.padding = Some(LengthSpec::Px(6.0));
                },
                (132.0, 92.0),
            ),
            (
                "min-height",
                |l| {
                    l.max_width = Some(LengthSpec::Px(60.0));
                    l.min_height = Some(LengthSpec::Px(50.0));
                },
                (60.0, 50.0),
            ),
            (
                "max-height",
                |l| l.max_height = Some(LengthSpec::Px(30.0)),
                (120.0, 30.0),
            ),
        ];
        for context in [
            Context::Flex,
            Context::Grid,
            Context::Inline,
            Context::Absolute,
            Context::AbsoluteInsets,
        ] {
            // What an in-flow container adds below its one item: nothing, or on
            // a line the strut below the inline-block's baseline.
            let mut line_slack = None;
            for (label, edit, expected) in cases {
                // Warm: a plain box first, then the case as an edit.
                let (mut warm, document, id) = page(context, ratio_box(|l| l.aspect_ratio = None));
                let mut queue = MutationQueue::new();
                queue.set_style(id, styled(context.item(ratio_box(edit))));
                warm.commit_mutations(queue).unwrap();
                product_frame(&mut warm, document, viewport(), &mut bundled_face_shaper());
                let bounds = warm.world().layout_box(id).unwrap();
                assert_eq!(
                    (bounds.width, bounds.height),
                    expected,
                    "{context:?} {label}"
                );
                // The container's auto height reads the item's measured height,
                // which the ratio fills from the limited width too.
                if !matches!(context, Context::Absolute | Context::AbsoluteInsets) {
                    let container = warm.world().parent_id(id).unwrap();
                    let slack = warm.world().layout_box(container).unwrap().height - bounds.height;
                    if !matches!(context, Context::Inline) {
                        assert_eq!(slack, 0.0, "{context:?} {label} container");
                    }
                    let first = *line_slack.get_or_insert(slack);
                    assert!(
                        (slack - first).abs() < 0.01,
                        "{context:?} {label} container: {slack} below, {first} in the first case"
                    );
                }
                let (mut cold, _, _) = page(context, ratio_box(edit));
                assert_matches_cold(&mut warm, &mut cold, document);
            }
        }
    }

    /// A shaper with the product UI face as `sans-serif`, and `Wide` in two
    /// faces of other metrics: Noto Sans Arabic up to weight 500, Noto Emoji
    /// above. With `late`, `Late` is Noto Sans Arabic as well.
    fn strut_shaper(late: bool) -> NanaTextEngineShaper {
        let mut policy = FallbackPolicy::empty();
        policy.set_generic(GenericFamily::SansSerif, ["Noto Sans SC"]);
        let mut fonts = FontSystem::with_policy(policy);
        fonts
            .register_file(ui_face(), &FaceDescriptor::default())
            .expect("the bundled UI face registers");
        for (file, weight) in [
            ("noto-sans-arabic.ttf", (100.0, 500.0)),
            ("noto-emoji.ttf", (600.0, 900.0)),
        ] {
            fonts
                .register_file(
                    corpus_face(file),
                    &FaceDescriptor {
                        weight: Some(weight),
                        ..FaceDescriptor::family("Wide")
                    },
                )
                .expect("a corpus face registers");
        }
        let shaper = NanaTextEngineShaper::new(Arc::new(Mutex::new(NativeTextEngine::new(fonts))));
        if late {
            register_late(&shaper);
        }
        shaper
    }

    fn ui_face() -> String {
        concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../nana-ui/assets/fonts/NotoSansSC-Variable.ttf"
        )
        .into()
    }

    fn corpus_face(file: &str) -> String {
        format!("{}/../nana-text/fonts/{file}", env!("CARGO_MANIFEST_DIR"))
    }

    /// Registers `Late`: a new font set, as a host's `@font-face` arriving.
    fn register_late(shaper: &NanaTextEngineShaper) {
        nana_text::lock_text_engine(shaper.engine())
            .fonts_mut()
            .register_file(
                corpus_face("noto-sans-arabic.ttf"),
                &FaceDescriptor::family("Late"),
            )
            .expect("a corpus face registers");
    }

    /// What the ordinary block in gate B is written in.
    #[derive(Debug, Clone, Copy)]
    struct Face {
        family: Option<&'static str>,
        weight: u16,
    }

    const FONT: f32 = 20.0;

    /// A page of an ordinary block (20px, `line-height: 20px`) holding a
    /// 40×30 and a 10×4 inline-block, a sibling block of two inline-blocks in
    /// the default font, and a flex row. `block_atoms` is the first atom's
    /// display. Returns the block, its first atom, the sibling and the row.
    fn strut_page(
        face: Face,
        first_atom: DisplaySpec,
        late: bool,
    ) -> (AppContext, NanaTextEngineShaper, [StableNodeId; 4]) {
        let document = DocumentId::new(1).unwrap();
        let (mut b, page) = Builder::page(document, 1, 800.0);
        let block = b.element(page, strut_block(face));
        let atom = b.element(block, strut_atom(first_atom, 40.0, 30.0));
        b.element(block, strut_atom(DisplaySpec::InlineBlock, 10.0, 4.0));
        let sibling = b.element(
            page,
            LayoutStyle {
                width: Some(LengthSpec::Px(600.0)),
                ..LayoutStyle::default()
            },
        );
        b.element(sibling, strut_atom(DisplaySpec::InlineBlock, 40.0, 30.0));
        b.element(sibling, strut_atom(DisplaySpec::InlineBlock, 20.0, 10.0));
        let row = b.element(
            page,
            LayoutStyle {
                display: Some(DisplaySpec::Flex),
                direction: Some(FlexDirection::Row),
                width: Some(LengthSpec::Px(200.0)),
                ..LayoutStyle::default()
            },
        );
        b.label(row, "Row");
        let mut context = AppContext::new();
        context.commit_mutations(b.queue).unwrap();
        let mut shaper = strut_shaper(late);
        product_frame(&mut context, document, viewport(), &mut shaper);
        (context, shaper, [block, atom, sibling, row])
    }

    fn strut_block(face: Face) -> LayoutStyle {
        LayoutStyle {
            display: Some(DisplaySpec::Block),
            width: Some(LengthSpec::Px(600.0)),
            font_size: Some(FONT),
            line_height: Some(nana_ui_core::LineHeightSpec::Absolute(FONT)),
            font_family: face.family.map(str::to_owned),
            font_weight: Some(face.weight),
            ..LayoutStyle::default()
        }
    }

    fn strut_atom(display: DisplaySpec, width: f32, height: f32) -> LayoutStyle {
        LayoutStyle {
            display: Some(display),
            width: Some(LengthSpec::Px(width)),
            height: Some(LengthSpec::Px(height)),
            ..LayoutStyle::default()
        }
    }

    /// Gate C: an atom of an ordinary block turned block-level, and back,
    /// keeps its box where its line put it, and is published as what it takes
    /// part as: a flow item, then an atomic inline box. Found by gate B, whose
    /// cold check reads the participation; it shows on this page and font set.
    #[test]
    fn issue382_c_a_display_change_republishes_the_participation() {
        let sans = Face {
            family: None,
            weight: 400,
        };
        let document = DocumentId::new(1).unwrap();
        let (mut warm, mut shaper, [_, atom, _, _]) =
            strut_page(sans, DisplaySpec::InlineBlock, false);
        for (display, part) in [
            (DisplaySpec::Block, crate::ParticipationKind::FlowItem),
            (
                DisplaySpec::InlineBlock,
                crate::ParticipationKind::AtomicInline,
            ),
        ] {
            let mut queue = MutationQueue::new();
            queue.set_style(atom, styled(strut_atom(display, 40.0, 30.0)));
            warm.commit_mutations(queue).unwrap();
            product_frame(&mut warm, document, viewport(), &mut shaper);
            let published = warm.world().layout_result(atom).unwrap().participation;
            assert_eq!(published, Some(part), "{display:?}");
            let (mut cold, _, _) = strut_page(sans, display, false);
            assert_matches_cold(&mut warm, &mut cold, document);
        }
    }

    /// Gate C, second part: every child of an ordinary block turned
    /// block-level and back in one commit. Block-level, the block stacks them
    /// and records a placement plan; inline-level again, it lays them on a line,
    /// which the plan does not model, and places them afresh.
    #[test]
    fn issue382_c_children_turned_inline_level_again_leave_the_flow_plan() {
        let child = |display, width, height| LayoutStyle {
            display: Some(display),
            width: Some(LengthSpec::Px(width)),
            height: Some(LengthSpec::Px(height)),
            ..LayoutStyle::default()
        };
        let build = |display| {
            let document = DocumentId::new(1).unwrap();
            let (mut b, page) = Builder::page(document, 1, 800.0);
            let block = b.element(
                page,
                LayoutStyle {
                    width: Some(LengthSpec::Px(600.0)),
                    ..LayoutStyle::default()
                },
            );
            let ids = [
                b.element(block, child(display, 40.0, 30.0)),
                b.element(block, child(display, 10.0, 4.0)),
            ];
            let mut app = AppContext::new();
            app.commit_mutations(b.queue).unwrap();
            product_frame(&mut app, document, viewport(), &mut bundled_face_shaper());
            (app, document, ids)
        };
        let (mut warm, document, [tall, short]) = build(DisplaySpec::InlineBlock);
        for display in [DisplaySpec::Block, DisplaySpec::InlineBlock] {
            let mut queue = MutationQueue::new();
            queue.set_style(tall, styled(child(display, 40.0, 30.0)));
            queue.set_style(short, styled(child(display, 10.0, 4.0)));
            warm.commit_mutations(queue).unwrap();
            product_frame(&mut warm, document, viewport(), &mut bundled_face_shaper());
            let (mut cold, _, _) = build(display);
            assert_matches_cold(&mut warm, &mut cold, document);
        }
        // On one line again: the short one beside the tall one.
        let (tall, short) = (
            warm.world().layout_box(tall).unwrap(),
            warm.world().layout_box(short).unwrap(),
        );
        assert_eq!(short.x, tall.x + tall.width, "{short:?} beside {tall:?}");
    }

    /// The face's own ascent and descent at `font` px, read the way the text
    /// pass reads them.
    fn face_strut(
        shaper: &mut NanaTextEngineShaper,
        family: Option<&str>,
        weight: u16,
        font: f32,
    ) -> FontStrut {
        let style = ComputedStyle {
            font_family: family.map(Arc::from),
            font_weight: Some(weight),
            font_size: font,
            ..ComputedStyle::default()
        };
        shaper
            .strut_metrics(&style)
            .expect("the face reports its metrics")
    }

    /// The block's line in `face`: the tall atom's bottom is the baseline, the
    /// strut's descent below it, so the block is 30 plus that descent tall
    /// (the line height is the font size, so the half-leading is negative).
    fn assert_line_in(
        context: &AppContext,
        block: StableNodeId,
        atom: StableNodeId,
        strut: FontStrut,
    ) {
        let world = context.world();
        let (line, tall) = (
            world.layout_box(block).unwrap(),
            world.layout_box(atom).unwrap(),
        );
        let half_leading = (FONT - (strut.ascent + strut.descent)) / 2.0;
        let below = strut.descent + half_leading;
        assert!(
            (line.height - (tall.height + below)).abs() < 0.01,
            "{line:?} with {tall:?}: {below} below the baseline"
        );
    }

    /// Gate B, first part: an ordinary block's line and a native paragraph's in
    /// the same font leave the same room below an empty atom, the face's own
    /// descent and half-leading, not the approximation's.
    #[test]
    fn issue382_b_a_block_line_and_a_paragraph_line_share_one_strut() {
        let document = DocumentId::new(1).unwrap();
        let (mut b, page) = Builder::page(document, 1, 800.0);
        let block = b.element(
            page,
            LayoutStyle {
                width: Some(LengthSpec::Px(600.0)),
                ..LayoutStyle::default()
            },
        );
        let atom = b.element(block, strut_atom(DisplaySpec::InlineBlock, 40.0, 30.0));
        let held = b.detached(strut_atom(DisplaySpec::InlineBlock, 40.0, 30.0));
        let mut context = AppContext::new();
        context.commit_mutations(b.queue).unwrap();
        let paragraph = context
            .create_component(
                document,
                InlineFlow::new(InlineContent::builder().child(held).build())
                    .width(LengthSpec::Px(600.0)),
            )
            .unwrap()
            .stable_id();
        let mut shaper = strut_shaper(false);
        product_frame(&mut context, document, viewport(), &mut shaper);
        // The paragraph sets the theme's body text; the block takes the same
        // size on the same line height.
        let text = context.world().computed_style(paragraph).cloned().unwrap();
        let (font, line_height) = (
            text.font_size,
            nana_ui_core::text_line_box_height_px(text.font_size, text.line_height),
        );
        let mut queue = MutationQueue::new();
        queue.set_style(
            block,
            styled(LayoutStyle {
                width: Some(LengthSpec::Px(600.0)),
                font_size: Some(font),
                line_height: Some(nana_ui_core::LineHeightSpec::Absolute(line_height)),
                ..LayoutStyle::default()
            }),
        );
        context.commit_mutations(queue).unwrap();
        product_frame(&mut context, document, viewport(), &mut shaper);
        let world = context.world();
        let gap = |line: StableNodeId, atom: StableNodeId| {
            let (line, atom) = (
                world.layout_box(line).unwrap(),
                world.layout_box(atom).unwrap(),
            );
            line.y + line.height - (atom.y + atom.height)
        };
        let (block_gap, paragraph_gap) = (gap(block, atom), gap(paragraph, held));
        assert!(
            (block_gap - paragraph_gap).abs() < 0.01,
            "block {block_gap} vs paragraph {paragraph_gap}"
        );
        // The face's descent and half-leading, not the approximation's.
        let strut = face_strut(&mut shaper, None, 400, font);
        let expected = strut.descent + (line_height - (strut.ascent + strut.descent)) / 2.0;
        assert!(
            (block_gap - expected).abs() < 0.01,
            "{block_gap} vs {expected}"
        );
        let approximation =
            font * (1.0 - nana_ui_core::TEXT_APPROX_ASCENT_EM) + (line_height - font) / 2.0;
        assert!(
            (block_gap - approximation).abs() > 1.0,
            "{block_gap} is the approximation"
        );
    }

    /// Gate B, second part: each input of the strut moves it, or leaves it,
    /// the way a cold layout of the same tree does.
    #[test]
    fn issue382_b_the_strut_follows_the_blocks_face() {
        let sans = Face {
            family: None,
            weight: 400,
        };
        let wide = Face {
            family: Some("Wide"),
            weight: 400,
        };
        let wide_bold = Face {
            family: Some("Wide"),
            weight: 700,
        };
        let late = Face {
            family: Some("Late, sans-serif"),
            weight: 400,
        };
        let document = DocumentId::new(1).unwrap();
        let (mut warm, mut shaper, [block, atom, sibling, row]) =
            strut_page(sans, DisplaySpec::InlineBlock, false);
        let mut probe = strut_shaper(true);
        assert_line_in(&warm, block, atom, face_strut(&mut probe, None, 400, FONT));
        let check = |warm: &mut AppContext, face: Face, display: DisplaySpec, late: bool| {
            let (mut cold, _, _) = strut_page(face, display, late);
            assert_matches_cold(warm, &mut cold, document);
        };

        // Another family, then another weight of it: other faces, other struts.
        for face in [wide, wide_bold] {
            let mut queue = MutationQueue::new();
            queue.set_style(block, styled(strut_block(face)));
            warm.commit_mutations(queue).unwrap();
            product_frame(&mut warm, document, viewport(), &mut shaper);
            assert_line_in(
                &warm,
                block,
                atom,
                face_strut(&mut probe, face.family, face.weight, FONT),
            );
            check(&mut warm, face, DisplaySpec::InlineBlock, false);
        }
        let held = warm.world().block_strut(block);

        // A language picks no other primary face: the strut and every box stay.
        let before = warm.world().layout_box(block);
        let mut queue = MutationQueue::new();
        queue.set_language(block, nana_text::font::LanguageTag::new("ja"));
        warm.commit_mutations(queue).unwrap();
        let (_, seeded) = product_frame_seeded(&mut warm, document, viewport(), &mut shaper);
        assert_eq!(warm.world().block_strut(block), held);
        assert_eq!(warm.world().layout_box(block), before);
        assert!(!seeded.contains(&block), "{seeded:?}");
        let mut queue = MutationQueue::new();
        queue.set_language(block, None);
        warm.commit_mutations(queue).unwrap();
        product_frame(&mut warm, document, viewport(), &mut shaper);

        // A family the font set lacks falls back; registering it is a new font
        // set, and the block's lines start from it.
        let mut queue = MutationQueue::new();
        queue.set_style(block, styled(strut_block(late)));
        warm.commit_mutations(queue).unwrap();
        product_frame(&mut warm, document, viewport(), &mut shaper);
        assert_line_in(
            &warm,
            block,
            atom,
            face_strut(&mut strut_shaper(false), None, 400, FONT),
        );
        register_late(&shaper);
        product_frame(&mut warm, document, viewport(), &mut shaper);
        assert_line_in(
            &warm,
            block,
            atom,
            face_strut(&mut probe, Some("Late"), 400, FONT),
        );
        check(&mut warm, late, DisplaySpec::InlineBlock, true);

        // An atom turned block-level leaves the block no inline-level child of
        // its own size: the line it shares with the other is still a line.
        // Both atoms block-level: no lines, no strut.
        for display in [DisplaySpec::Block, DisplaySpec::InlineBlock] {
            let mut queue = MutationQueue::new();
            queue.set_style(atom, styled(strut_atom(display, 40.0, 30.0)));
            warm.commit_mutations(queue).unwrap();
            product_frame(&mut warm, document, viewport(), &mut shaper);
            assert!(warm.world().block_strut(block).is_some());
            check(&mut warm, late, display, true);
        }
        let short = warm.world().node(block).unwrap().children[1];
        let mut queue = MutationQueue::new();
        for id in [atom, short] {
            queue.set_style(id, styled(strut_atom(DisplaySpec::Block, 40.0, 30.0)));
        }
        warm.commit_mutations(queue).unwrap();
        product_frame(&mut warm, document, viewport(), &mut shaper);
        assert_eq!(warm.world().block_strut(block), None);
        let mut queue = MutationQueue::new();
        queue.set_style(
            short,
            styled(strut_atom(DisplaySpec::InlineBlock, 10.0, 4.0)),
        );
        queue.set_style(
            atom,
            styled(strut_atom(DisplaySpec::InlineBlock, 40.0, 30.0)),
        );
        warm.commit_mutations(queue).unwrap();
        product_frame(&mut warm, document, viewport(), &mut shaper);
        check(&mut warm, late, DisplaySpec::InlineBlock, true);

        // Changes that do not touch a strut lay no block out for one: the
        // flex row's label, and a font size (the strut is per px of it).
        let mut queue = MutationQueue::new();
        let label = warm.world().node(row).unwrap().children[0];
        queue.set_text(label, crate::TextContent::new("A longer row label"));
        warm.commit_mutations(queue).unwrap();
        let (_, seeded) = product_frame_seeded(&mut warm, document, viewport(), &mut shaper);
        for id in [block, sibling] {
            assert!(!seeded.contains(&id), "{id:?} in {seeded:?}");
            assert!(!measured(&warm, id), "{id:?}");
        }
        let held = warm.world().block_strut(block);
        let mut queue = MutationQueue::new();
        queue.set_style(
            block,
            styled(LayoutStyle {
                font_size: Some(FONT * 1.5),
                ..strut_block(late)
            }),
        );
        warm.commit_mutations(queue).unwrap();
        product_frame(&mut warm, document, viewport(), &mut shaper);
        assert_eq!(warm.world().block_strut(block), held);

        // The block's last inline-level child leaves: no lines, no strut; one
        // arrives: the strut is back. Child list edits alone, with no style of
        // the block's changing.
        let mut queue = MutationQueue::new();
        queue.set_style(atom, styled(strut_atom(DisplaySpec::Block, 40.0, 30.0)));
        queue.despawn_subtree(short);
        warm.commit_mutations(queue).unwrap();
        product_frame(&mut warm, document, viewport(), &mut shaper);
        assert_eq!(warm.world().block_strut(block), None);
        let arrived = StableNodeId::new(900).unwrap();
        let mut queue = MutationQueue::new();
        queue.create(arrived, document, NodeKind::Element { tag: "div".into() });
        queue.insert(block, arrived, None);
        queue.set_style(
            arrived,
            styled(strut_atom(DisplaySpec::InlineBlock, 10.0, 4.0)),
        );
        warm.commit_mutations(queue).unwrap();
        product_frame(&mut warm, document, viewport(), &mut shaper);
        assert_eq!(warm.world().block_strut(block), held);
    }

    /// Gate B, third part: what the block inherits and what it holds. Its face
    /// comes from a wrapper it inherits from, and its only inline-level child
    /// moves out and back with no style changing on either.
    #[test]
    fn issue382_b_an_inherited_face_and_a_moved_child_reach_the_strut() {
        let wrapper = |family: Option<&str>| LayoutStyle {
            width: Some(LengthSpec::Px(600.0)),
            font_family: family.map(str::to_owned),
            ..LayoutStyle::default()
        };
        let inheriting = || LayoutStyle {
            font_family: None,
            font_weight: None,
            ..strut_block(Face {
                family: None,
                weight: 400,
            })
        };
        let build = |family: Option<&'static str>, held: bool| {
            let document = DocumentId::new(1).unwrap();
            let (mut b, page) = Builder::page(document, 1, 800.0);
            let outer = b.element(page, wrapper(family));
            let block = b.element(outer, inheriting());
            b.element(block, strut_atom(DisplaySpec::Block, 40.0, 30.0));
            let atom = b.element(
                if held { block } else { page },
                strut_atom(DisplaySpec::InlineBlock, 10.0, 4.0),
            );
            let mut context = AppContext::new();
            context.commit_mutations(b.queue).unwrap();
            let mut shaper = strut_shaper(false);
            product_frame(&mut context, document, viewport(), &mut shaper);
            (context, shaper, [page, outer, block, atom])
        };
        let document = DocumentId::new(1).unwrap();
        let (mut warm, mut shaper, [page, outer, block, atom]) = build(None, true);
        let mut probe = strut_shaper(false);
        let per_px = |strut: FontStrut| FontStrut {
            ascent: strut.ascent / FONT,
            descent: strut.descent / FONT,
        };
        assert_eq!(
            warm.world().block_strut(block),
            Some(per_px(face_strut(&mut probe, None, 400, FONT)))
        );

        // The wrapper's family reaches the block through inheritance alone.
        let mut queue = MutationQueue::new();
        queue.set_style(outer, styled(wrapper(Some("Wide"))));
        warm.commit_mutations(queue).unwrap();
        product_frame(&mut warm, document, viewport(), &mut shaper);
        assert_eq!(
            warm.world().block_strut(block),
            Some(per_px(face_strut(&mut probe, Some("Wide"), 400, FONT)))
        );
        let (mut cold, _, _) = build(Some("Wide"), true);
        assert_matches_cold(&mut warm, &mut cold, document);

        // The only inline-level child moves out: the block lays no line out
        // and holds no strut. Back in, it does again.
        for held in [false, true] {
            let mut queue = MutationQueue::new();
            queue.insert(if held { block } else { page }, atom, None);
            warm.commit_mutations(queue).unwrap();
            product_frame(&mut warm, document, viewport(), &mut shaper);
            assert_eq!(warm.world().block_strut(block).is_some(), held);
            let (mut cold, _, _) = build(Some("Wide"), held);
            assert_matches_cold(&mut warm, &mut cold, document);
        }
    }

    /// What gate D aligns: a label, a fixed box whose first line box is a
    /// label below a spacer, a box whose first line box is two blocks down,
    /// and a narrow box whose two empty atoms wrap to two lines, on a flex row
    /// that aligns them by their first baselines. Returns the row, the labels,
    /// the boxed text and spacer, the nested box's second label, a label
    /// outside the row, the fixed box and the narrow box's two atoms.
    fn first_baseline_page(
        boxed_font: f32,
        spacer: f32,
        second: &str,
        outside: &str,
    ) -> (AppContext, NanaTextEngineShaper, [StableNodeId; 11]) {
        let document = DocumentId::new(1).unwrap();
        let (mut b, page) = Builder::page(document, 1, 800.0);
        let row = b.element(
            page,
            LayoutStyle {
                display: Some(DisplaySpec::Flex),
                direction: Some(FlexDirection::Row),
                align_items: nana_ui_core::AlignSpec::Baseline,
                width: Some(LengthSpec::Px(700.0)),
                ..LayoutStyle::default()
            },
        );
        let plain = b.label(row, "Plain");
        let boxed = b.element(
            row,
            LayoutStyle {
                width: Some(LengthSpec::Px(120.0)),
                height: Some(LengthSpec::Px(80.0)),
                padding_top: Some(LengthSpec::Px(7.0)),
                ..LayoutStyle::default()
            },
        );
        let boxed_spacer = b.element(
            boxed,
            LayoutStyle {
                height: Some(LengthSpec::Px(spacer)),
                ..LayoutStyle::default()
            },
        );
        let boxed_text = b.element(
            boxed,
            LayoutStyle {
                font_size: Some(boxed_font),
                ..LayoutStyle::default()
            },
        );
        let boxed_label = b.label(boxed_text, "Boxed");
        let nested = b.element(
            row,
            LayoutStyle {
                width: Some(LengthSpec::Px(120.0)),
                ..LayoutStyle::default()
            },
        );
        let inner = b.element(nested, LayoutStyle::default());
        let nested_label = b.label(inner, "Nested");
        let nested_second = b.label(nested, second);
        let narrow = b.element(
            row,
            LayoutStyle {
                width: Some(LengthSpec::Px(60.0)),
                ..LayoutStyle::default()
            },
        );
        let lines = [
            b.element(narrow, strut_atom(DisplaySpec::InlineBlock, 40.0, 20.0)),
            b.element(narrow, strut_atom(DisplaySpec::InlineBlock, 40.0, 20.0)),
        ];
        let outside = b.label(page, outside);
        let mut context = AppContext::new();
        context.commit_mutations(b.queue).unwrap();
        let mut shaper = strut_shaper(false);
        product_frame(&mut context, document, viewport(), &mut shaper);
        (
            context,
            shaper,
            [
                row,
                plain,
                boxed_label,
                boxed_text,
                boxed_spacer,
                nested_label,
                nested_second,
                outside,
                boxed,
                lines[0],
                lines[1],
            ],
        )
    }

    /// Every first line on the row sits on one baseline: the plain label's,
    /// the boxed label's below its spacer, the nested label's, and the narrow
    /// box's first line, which its empty atom sits on by its bottom edge; its
    /// second line is below.
    fn assert_on_one_baseline(
        context: &AppContext,
        labels: [StableNodeId; 3],
        lines: [StableNodeId; 2],
    ) {
        let world = context.world();
        let baselines = labels.map(|label| {
            world
                .layout_result(label)
                .and_then(|result| result.first_baseline)
                .unwrap_or_else(|| panic!("{label:?} publishes no baseline"))
        });
        let bottom = |id| {
            let atom = world.layout_box(id).unwrap();
            atom.y + atom.height
        };
        for baseline in baselines.into_iter().chain([bottom(lines[0])]) {
            assert!(
                (baseline - baselines[0]).abs() < 0.01,
                "first baselines {baselines:?}, first atom's bottom {}",
                bottom(lines[0])
            );
        }
        assert!(
            bottom(lines[1]) > bottom(lines[0]) + 10.0,
            "the narrow box holds one line"
        );
    }

    /// Gate D: an item of a baseline-aligned flex row whose line boxes are its
    /// descendants' sits by its first in-flow line box, wherever that lies
    /// inside it: below a spacer in a fixed box, two blocks down. A new font
    /// size in the fixed box and a taller spacer align the row again; text that
    /// leaves the first line where it was, and text outside the row, move
    /// nothing. Each step matches a cold layout.
    #[test]
    fn issue382_d_a_flex_item_sits_by_its_first_descendant_line() {
        let document = DocumentId::new(1).unwrap();
        let (mut warm, mut shaper, ids) = first_baseline_page(16.0, 10.0, "Second", "Outside");
        let [
            row,
            plain,
            boxed_label,
            boxed_text,
            boxed_spacer,
            nested_label,
            nested_second,
            outside,
            boxed,
            first_line,
            second_line,
        ] = ids;
        let lines = [first_line, second_line];
        let labels = [plain, boxed_label, nested_label];
        assert_on_one_baseline(&warm, labels, lines);
        let (mut cold, _, _) = first_baseline_page(16.0, 10.0, "Second", "Outside");
        assert_matches_cold(&mut warm, &mut cold, document);
        let fixed = warm.world().layout_box(boxed);

        // A bigger font in the fixed box: its first line moves down inside it,
        // the box does not grow, the row aligns again.
        let mut queue = MutationQueue::new();
        queue.set_style(
            boxed_text,
            styled(LayoutStyle {
                font_size: Some(28.0),
                ..LayoutStyle::default()
            }),
        );
        warm.commit_mutations(queue).unwrap();
        product_frame(&mut warm, document, viewport(), &mut shaper);
        assert_eq!(
            warm.world().layout_box(boxed).map(|b| (b.width, b.height)),
            fixed.map(|b| (b.width, b.height))
        );
        assert_on_one_baseline(&warm, labels, lines);
        let (mut cold, _, _) = first_baseline_page(28.0, 10.0, "Second", "Outside");
        assert_matches_cold(&mut warm, &mut cold, document);

        // A taller spacer above the first line in the fixed box.
        let mut queue = MutationQueue::new();
        queue.set_style(
            boxed_spacer,
            styled(LayoutStyle {
                height: Some(LengthSpec::Px(24.0)),
                ..LayoutStyle::default()
            }),
        );
        warm.commit_mutations(queue).unwrap();
        product_frame(&mut warm, document, viewport(), &mut shaper);
        assert_on_one_baseline(&warm, labels, lines);
        let (mut cold, _, _) = first_baseline_page(28.0, 24.0, "Second", "Outside");
        assert_matches_cold(&mut warm, &mut cold, document);

        // Text below the nested box's first line, and text outside the row:
        // no item of the row moves.
        let items = |context: &AppContext| {
            let world = context.world();
            world
                .node(row)
                .unwrap()
                .children
                .iter()
                .map(|id| world.layout_box(*id))
                .collect::<Vec<_>>()
        };
        let before = items(&warm);
        for (id, text) in [
            (nested_second, "Second, longer"),
            (outside, "Outside, longer"),
        ] {
            let mut queue = MutationQueue::new();
            queue.set_text(id, crate::TextContent::new(text));
            warm.commit_mutations(queue).unwrap();
            product_frame(&mut warm, document, viewport(), &mut shaper);
            assert_eq!(items(&warm), before, "{text}");
        }
        assert!(
            !measured(&warm, row),
            "the row measured for text outside it"
        );
        let (mut cold, _, _) = first_baseline_page(28.0, 24.0, "Second, longer", "Outside, longer");
        assert_matches_cold(&mut warm, &mut cold, document);
    }

    /// The block flows gate E lays out: two labels in a fixed box whose flow
    /// puts its free space before, between or around them, or stacks them up
    /// from the bottom.
    const FLOWS: [(JustifySpec, bool); 6] = [
        (JustifySpec::Center, false),
        (JustifySpec::End, false),
        (JustifySpec::SpaceBetween, false),
        (JustifySpec::SpaceEvenly, false),
        (JustifySpec::Start, true),
        (JustifySpec::End, true),
    ];

    /// A flex row aligning by first baselines and an ordinary block's line,
    /// each holding a reference label and one fixed box per [`FLOWS`] entry,
    /// the first entry's justification replaced by `first`. Returns the row's
    /// and the line's labels, the reference first, then each box's two, and
    /// the row's and the line's first box.
    fn flow_baseline_page(
        first: JustifySpec,
    ) -> (
        AppContext,
        NanaTextEngineShaper,
        [Vec<StableNodeId>; 2],
        [StableNodeId; 2],
    ) {
        let document = DocumentId::new(1).unwrap();
        let (mut b, page) = Builder::page(document, 1, 900.0);
        let row = b.element(
            page,
            LayoutStyle {
                display: Some(DisplaySpec::Flex),
                direction: Some(FlexDirection::Row),
                align_items: nana_ui_core::AlignSpec::Baseline,
                width: Some(LengthSpec::Px(800.0)),
                ..LayoutStyle::default()
            },
        );
        let line = b.element(
            page,
            LayoutStyle {
                width: Some(LengthSpec::Px(800.0)),
                ..LayoutStyle::default()
            },
        );
        let mut labels = [Vec::new(), Vec::new()];
        let mut first_boxes = Vec::new();
        for (slot, parent) in [row, line].into_iter().enumerate() {
            let reference = b.element(
                parent,
                LayoutStyle {
                    display: Some(DisplaySpec::InlineBlock),
                    ..LayoutStyle::default()
                },
            );
            labels[slot].push(b.label(reference, "Ref"));
            for (index, (justify, reversed)) in FLOWS.into_iter().enumerate() {
                let flow = b.element(
                    parent,
                    LayoutStyle {
                        display: Some(DisplaySpec::InlineBlock),
                        width: Some(LengthSpec::Px(90.0)),
                        height: Some(LengthSpec::Px(80.0)),
                        justify_content: if index == 0 { first } else { justify },
                        flex_reverse: reversed,
                        ..LayoutStyle::default()
                    },
                );
                if index == 0 {
                    first_boxes.push(flow);
                }
                for text in ["A", "B"] {
                    let holder = b.element(flow, LayoutStyle::default());
                    labels[slot].push(b.label(holder, text));
                }
            }
        }
        let mut context = AppContext::new();
        context.commit_mutations(b.queue).unwrap();
        let mut shaper = strut_shaper(false);
        product_frame(&mut context, document, viewport(), &mut shaper);
        (context, shaper, labels, [first_boxes[0], first_boxes[1]])
    }

    /// The row sits each box by its first label in flow order, the line by its
    /// last: each on the reference label's baseline, wherever its flow put it.
    fn assert_flows_on_one_baseline(context: &AppContext, labels: &[Vec<StableNodeId>; 2]) {
        let world = context.world();
        let baseline = |id: StableNodeId| {
            world
                .layout_result(id)
                .and_then(|result| result.first_baseline)
                .unwrap_or_else(|| panic!("{id:?} publishes no baseline"))
        };
        for (slot, labels) in labels.iter().enumerate() {
            let reference = baseline(labels[0]);
            for (index, pair) in labels[1..].chunks(2).enumerate() {
                let sitting = pair[if slot == 0 { 0 } else { 1 }];
                let other = pair[if slot == 0 { 1 } else { 0 }];
                assert!(
                    (baseline(sitting) - reference).abs() < 0.01,
                    "{} {:?}: {} vs the reference {reference}",
                    if slot == 0 { "row" } else { "line" },
                    FLOWS[index],
                    baseline(sitting)
                );
                assert!(
                    (baseline(other) - reference).abs() > 1.0,
                    "{:?}: both labels on the baseline",
                    FLOWS[index]
                );
            }
        }
    }

    /// Gate E: a block flow that is not packed to its start sits by where its
    /// first and last line boxes are: `justify-content` center, end,
    /// space-between and space-evenly, and a reversed flow, alone and packed
    /// to its end. On a flex row aligning first baselines and on an ordinary
    /// block's line (last baselines) alike; a new justification aligns them
    /// again. Each step matches a cold layout.
    #[test]
    fn issue382_e_a_justified_or_reversed_block_flow_sits_by_its_line_boxes() {
        let document = DocumentId::new(1).unwrap();
        let (mut warm, mut shaper, labels, first_boxes) = flow_baseline_page(JustifySpec::Center);
        assert_flows_on_one_baseline(&warm, &labels);
        let (mut cold, _, _, _) = flow_baseline_page(JustifySpec::Center);
        assert_matches_cold(&mut warm, &mut cold, document);
        let mut queue = MutationQueue::new();
        for first_box in first_boxes {
            queue.set_style(
                first_box,
                styled(LayoutStyle {
                    display: Some(DisplaySpec::InlineBlock),
                    width: Some(LengthSpec::Px(90.0)),
                    height: Some(LengthSpec::Px(80.0)),
                    justify_content: JustifySpec::End,
                    ..LayoutStyle::default()
                }),
            );
        }
        warm.commit_mutations(queue).unwrap();
        product_frame(&mut warm, document, viewport(), &mut shaper);
        assert_flows_on_one_baseline(&warm, &labels);
        let (mut cold, _, _, _) = flow_baseline_page(JustifySpec::End);
        assert_matches_cold(&mut warm, &mut cold, document);
    }

    /// A flex row aligning first baselines and an ordinary block's line, each
    /// with a reference label and a 100-wide box that floats an 80×40 box
    /// left: on the row its one 40×20 atom does not fit beside the float and
    /// drops below it; on the line three such atoms drop below it and wrap.
    /// `float` floats the box (or leaves it in flow, 80 wide). Returns the
    /// references and the atoms the box sits by: the row's only one, the
    /// line's last.
    fn float_baseline_page(float: bool) -> (AppContext, NanaTextEngineShaper, [StableNodeId; 4]) {
        let document = DocumentId::new(1).unwrap();
        let (mut b, page) = Builder::page(document, 1, 900.0);
        let row = b.element(
            page,
            LayoutStyle {
                display: Some(DisplaySpec::Flex),
                direction: Some(FlexDirection::Row),
                align_items: nana_ui_core::AlignSpec::Baseline,
                width: Some(LengthSpec::Px(800.0)),
                ..LayoutStyle::default()
            },
        );
        let line = b.element(
            page,
            LayoutStyle {
                width: Some(LengthSpec::Px(800.0)),
                ..LayoutStyle::default()
            },
        );
        let mut ids = Vec::new();
        for (parent, atoms) in [(row, 1), (line, 3)] {
            let reference = b.element(
                parent,
                LayoutStyle {
                    display: Some(DisplaySpec::InlineBlock),
                    ..LayoutStyle::default()
                },
            );
            ids.push(b.label(reference, "Ref"));
            let holder = b.element(
                parent,
                LayoutStyle {
                    display: Some(DisplaySpec::InlineBlock),
                    width: Some(LengthSpec::Px(100.0)),
                    ..LayoutStyle::default()
                },
            );
            b.element(
                holder,
                LayoutStyle {
                    width: Some(LengthSpec::Px(80.0)),
                    height: Some(LengthSpec::Px(40.0)),
                    float: if float {
                        nana_ui_core::FloatSpec::Left
                    } else {
                        nana_ui_core::FloatSpec::None
                    },
                    ..LayoutStyle::default()
                },
            );
            let mut last = None;
            for _ in 0..atoms {
                last = Some(b.element(holder, strut_atom(DisplaySpec::InlineBlock, 40.0, 20.0)));
            }
            ids.push(last.unwrap());
        }
        let mut context = AppContext::new();
        context.commit_mutations(b.queue).unwrap();
        let mut shaper = strut_shaper(false);
        product_frame(&mut context, document, viewport(), &mut shaper);
        (context, shaper, [ids[0], ids[1], ids[2], ids[3]])
    }

    /// Gate F: a box whose lines run beside a float sits by where those lines
    /// are: on a flex row by its first line, which an atom too wide for the
    /// room beside the float takes below it; on a line by its last, which
    /// wraps below the float. Floating the box and sinking it back into the
    /// flow each place them again, as a cold layout does.
    #[test]
    fn issue382_f_lines_beside_a_float_give_the_baselines() {
        let document = DocumentId::new(1).unwrap();
        let on_baseline =
            |context: &AppContext, [row_ref, row_atom, line_ref, line_atom]: [StableNodeId; 4]| {
                let world = context.world();
                for (reference, atom) in [(row_ref, row_atom), (line_ref, line_atom)] {
                    let baseline = world
                        .layout_result(reference)
                        .and_then(|result| result.first_baseline)
                        .unwrap();
                    let atom = world.layout_box(atom).unwrap();
                    assert!(
                        (atom.y + atom.height - baseline).abs() < 0.01,
                        "{atom:?} sits off the baseline {baseline}"
                    );
                }
            };
        let (mut warm, mut shaper, ids) = float_baseline_page(true);
        on_baseline(&warm, ids);
        // Below the float: the first line starts at its bottom.
        let holder = warm.world().parent_id(ids[1]).unwrap();
        let (atom, holder_box) = (
            warm.world().layout_box(ids[1]).unwrap(),
            warm.world().layout_box(holder).unwrap(),
        );
        assert!(
            atom.y >= holder_box.y + 40.0 - 0.01,
            "{atom:?} in {holder_box:?}"
        );
        let (mut cold, _, _) = float_baseline_page(true);
        assert_matches_cold(&mut warm, &mut cold, document);
        for float in [false, true] {
            let mut queue = MutationQueue::new();
            for atom in [ids[1], ids[3]] {
                let holder = warm.world().parent_id(atom).unwrap();
                let box_id = warm.world().node(holder).unwrap().children[0];
                queue.set_style(
                    box_id,
                    styled(LayoutStyle {
                        width: Some(LengthSpec::Px(80.0)),
                        height: Some(LengthSpec::Px(40.0)),
                        float: if float {
                            nana_ui_core::FloatSpec::Left
                        } else {
                            nana_ui_core::FloatSpec::None
                        },
                        ..LayoutStyle::default()
                    }),
                );
            }
            warm.commit_mutations(queue).unwrap();
            product_frame(&mut warm, document, viewport(), &mut shaper);
            on_baseline(&warm, ids);
            let (mut cold, _, _) = float_baseline_page(float);
            assert_matches_cold(&mut warm, &mut cold, document);
        }
    }

    /// The wrapping block flows gate G lays out: four 25-tall labelled blocks
    /// in a 60-tall flow, two to a column, packed to the end or reversed.
    const WRAPPED: [(JustifySpec, bool); 2] =
        [(JustifySpec::End, false), (JustifySpec::Start, true)];

    /// A flex row aligning first baselines and an ordinary block's line, each
    /// with a reference label and one wrapping flow per [`WRAPPED`] entry.
    /// Returns the row's and the line's labels: the reference, then each
    /// flow's four in flow order.
    fn wrap_baseline_page(
        height: f32,
    ) -> (AppContext, NanaTextEngineShaper, [Vec<StableNodeId>; 2]) {
        let document = DocumentId::new(1).unwrap();
        let (mut b, page) = Builder::page(document, 1, 900.0);
        let row = b.element(
            page,
            LayoutStyle {
                display: Some(DisplaySpec::Flex),
                direction: Some(FlexDirection::Row),
                align_items: nana_ui_core::AlignSpec::Baseline,
                width: Some(LengthSpec::Px(800.0)),
                ..LayoutStyle::default()
            },
        );
        let line = b.element(
            page,
            LayoutStyle {
                width: Some(LengthSpec::Px(800.0)),
                ..LayoutStyle::default()
            },
        );
        let mut labels = [Vec::new(), Vec::new()];
        for (slot, parent) in [row, line].into_iter().enumerate() {
            let reference = b.element(
                parent,
                LayoutStyle {
                    display: Some(DisplaySpec::InlineBlock),
                    ..LayoutStyle::default()
                },
            );
            labels[slot].push(b.label(reference, "Ref"));
            for (justify, reversed) in WRAPPED {
                let flow = b.element(
                    parent,
                    LayoutStyle {
                        display: Some(DisplaySpec::InlineBlock),
                        width: Some(LengthSpec::Px(160.0)),
                        height: Some(LengthSpec::Px(height)),
                        flex_wrap: nana_ui_core::FlexWrap::Wrap,
                        justify_content: justify,
                        flex_reverse: reversed,
                        ..LayoutStyle::default()
                    },
                );
                for text in ["A", "B", "C", "D"] {
                    let holder = b.element(
                        flow,
                        LayoutStyle {
                            width: Some(LengthSpec::Px(60.0)),
                            height: Some(LengthSpec::Px(25.0)),
                            ..LayoutStyle::default()
                        },
                    );
                    labels[slot].push(b.label(holder, text));
                }
            }
        }
        let mut context = AppContext::new();
        context.commit_mutations(b.queue).unwrap();
        let mut shaper = strut_shaper(false);
        product_frame(&mut context, document, viewport(), &mut shaper);
        (context, shaper, labels)
    }

    /// Gate G: a block flow that wraps into columns sits by where its first and
    /// last line boxes are, in the columns placement lays them in: packed to
    /// the end of each column, or reversed. On a flex row (first baselines,
    /// the first label) and on a line (last baselines, the fourth); a taller
    /// flow puts all four in one column and aligns them again. Each step
    /// matches a cold layout.
    #[test]
    fn issue382_g_a_wrapping_block_flow_sits_by_its_columns_line_boxes() {
        let document = DocumentId::new(1).unwrap();
        let check = |context: &AppContext, labels: &[Vec<StableNodeId>; 2], columns: bool| {
            let world = context.world();
            let baseline = |id: StableNodeId| {
                world
                    .layout_result(id)
                    .and_then(|result| result.first_baseline)
                    .unwrap()
            };
            for (slot, labels) in labels.iter().enumerate() {
                let reference = baseline(labels[0]);
                for (index, flow) in labels[1..].chunks(4).enumerate() {
                    let sitting = flow[if slot == 0 { 0 } else { 3 }];
                    assert!(
                        (baseline(sitting) - reference).abs() < 0.01,
                        "{} {:?}: {} vs the reference {reference}",
                        if slot == 0 { "row" } else { "line" },
                        WRAPPED[index],
                        baseline(sitting)
                    );
                    let (first, third) = (
                        world.layout_box(flow[0]).unwrap(),
                        world.layout_box(flow[2]).unwrap(),
                    );
                    assert_eq!(
                        third.x > first.x + 30.0,
                        columns,
                        "{:?}: {first:?}, {third:?}",
                        WRAPPED[index]
                    );
                }
            }
        };
        let (mut warm, mut shaper, labels) = wrap_baseline_page(60.0);
        check(&warm, &labels, true);
        let (mut cold, _, _) = wrap_baseline_page(60.0);
        assert_matches_cold(&mut warm, &mut cold, document);
        for height in [120.0, 60.0] {
            let mut queue = MutationQueue::new();
            for slot in &labels {
                for flow in slot[1..].chunks(4) {
                    let holder = warm.world().parent_id(flow[0]).unwrap();
                    let flow_box = warm.world().parent_id(holder).unwrap();
                    let index = warm
                        .world()
                        .node(warm.world().parent_id(flow_box).unwrap())
                        .unwrap()
                        .children
                        .iter()
                        .position(|id| *id == flow_box)
                        .unwrap()
                        - 1;
                    let (justify, reversed) = WRAPPED[index];
                    queue.set_style(
                        flow_box,
                        styled(LayoutStyle {
                            display: Some(DisplaySpec::InlineBlock),
                            width: Some(LengthSpec::Px(160.0)),
                            height: Some(LengthSpec::Px(height)),
                            flex_wrap: nana_ui_core::FlexWrap::Wrap,
                            justify_content: justify,
                            flex_reverse: reversed,
                            ..LayoutStyle::default()
                        }),
                    );
                }
            }
            warm.commit_mutations(queue).unwrap();
            product_frame(&mut warm, document, viewport(), &mut shaper);
            check(&warm, &labels, height < 100.0);
            let (mut cold, _, _) = wrap_baseline_page(height);
            assert_matches_cold(&mut warm, &mut cold, document);
        }
    }
}

mod hidden_withdrawal {
    //! Issue #386: a node with no box (`hidden` / `display: none`, or inside
    //! such a node) publishes no `LayoutResult`, however its ancestors were laid
    //! out before, as a cold layout of the same tree publishes none.

    use std::sync::Arc;

    use nana_ui_core::{DisplaySpec, FlexDirection, LayoutStyle, ThemeDefinition};

    use crate::world::reflow_oracle::{
        Builder, assert_matches_cold, bundled_face_shaper, product_frame, styled,
    };
    use crate::{AppContext, DocumentId, LayoutViewport, MutationQueue, StableNodeId};

    fn viewport() -> LayoutViewport {
        LayoutViewport::new(600.0, 400.0)
    }

    fn row(display: DisplaySpec) -> LayoutStyle {
        LayoutStyle {
            display: Some(display),
            direction: Some(FlexDirection::Row),
            ..LayoutStyle::default()
        }
    }

    fn hidden(hidden: bool) -> LayoutStyle {
        LayoutStyle {
            hidden,
            ..LayoutStyle::default()
        }
    }

    /// A row holding a visible text, a hidden text and a hidden box with a text
    /// inside, under `theme` when given, laid out once. Returns the row and the
    /// hidden text, the hidden box and the text in it.
    fn page(
        display: DisplaySpec,
        theme: Option<&ThemeDefinition>,
    ) -> (AppContext, StableNodeId, [StableNodeId; 3]) {
        let document = DocumentId::new(1).unwrap();
        let mut cx = AppContext::new();
        if let Some(theme) = theme {
            cx.set_theme_definition(theme).unwrap();
        }
        let (mut b, page) = Builder::page(document, 1, 400.0);
        let row = b.element(page, row(display));
        b.label(row, "Shown");
        let text = b.label(row, "Hidden");
        b.queue.set_style(text, styled(hidden(true)));
        let boxed = b.element(row, hidden(true));
        let inner = b.label(boxed, "Inside");
        cx.commit_mutations(b.queue).unwrap();
        product_frame(&mut cx, document, viewport(), &mut bundled_face_shaper());
        (cx, row, [text, boxed, inner])
    }

    fn published(cx: &AppContext, ids: [StableNodeId; 3]) -> [bool; 3] {
        ids.map(|id| cx.world().layout_result(id).is_some())
    }

    /// A theme switch relays the row out; the hidden nodes still have no result.
    #[test]
    fn issue386_hidden_nodes_publish_nothing_after_a_theme_switch() {
        let document = DocumentId::new(1).unwrap();
        let theme = ThemeDefinition::NANA_DARK;
        let (mut cx, _, ids) = page(DisplaySpec::Flex, None);
        assert_eq!(published(&cx, ids), [false; 3]);
        cx.set_theme_definition(&theme).unwrap();
        product_frame(&mut cx, document, viewport(), &mut bundled_face_shaper());
        assert_eq!(published(&cx, ids), [false; 3]);
        let (mut cold, ..) = page(DisplaySpec::Flex, Some(&theme));
        assert_matches_cold(&mut cx, &mut cold, document);
    }

    /// The row switching between Flex and Grid keeps the hidden nodes out of
    /// the published results.
    #[test]
    fn issue386_hidden_nodes_publish_nothing_across_context_switches() {
        let document = DocumentId::new(1).unwrap();
        let (mut cx, row, ids) = page(DisplaySpec::Flex, None);
        for display in [DisplaySpec::Grid, DisplaySpec::Flex] {
            let mut style = cx.world().node_style(row).unwrap().clone();
            Arc::make_mut(&mut style.layout).display = Some(display);
            let mut q = MutationQueue::new();
            q.set_style(row, style);
            cx.commit_mutations(q).unwrap();
            product_frame(&mut cx, document, viewport(), &mut bundled_face_shaper());
            assert_eq!(published(&cx, ids), [false; 3], "{display:?}");
            let (mut cold, ..) = page(display, None);
            assert_matches_cold(&mut cx, &mut cold, document);
        }
    }

    /// A text hidden after it was laid out drops its result, and gets one back
    /// when shown again, as a cold layout does.
    #[test]
    fn issue386_hiding_a_laid_out_text_retracts_its_result() {
        let document = DocumentId::new(1).unwrap();
        let (mut cx, _, [text, ..]) = page(DisplaySpec::Flex, None);
        for shown in [true, false] {
            let mut q = MutationQueue::new();
            q.set_style(text, styled(hidden(!shown)));
            cx.commit_mutations(q).unwrap();
            product_frame(&mut cx, document, viewport(), &mut bundled_face_shaper());
            assert_eq!(cx.world().layout_result(text).is_some(), shown);
        }
        let (mut cold, ..) = page(DisplaySpec::Flex, None);
        assert_matches_cold(&mut cx, &mut cold, document);
    }

    /// A closed menu's items lay out with no box although their own style
    /// keeps one: after a theme switch they publish no result either.
    #[test]
    fn issue386_a_closed_menus_items_publish_nothing_after_a_theme_switch() {
        use crate::{ActionMenuItem, Popover, Stack};
        let document = DocumentId::new(1).unwrap();
        let theme = ThemeDefinition::NANA_DARK;
        let build = |theme: Option<&ThemeDefinition>| {
            let mut cx = AppContext::new();
            if let Some(theme) = theme {
                cx.set_theme_definition(theme).unwrap();
            }
            let root = cx.create_component(document, Stack::row(4.0)).unwrap();
            let menu = cx
                .create_detached_component(document, Popover::new().trigger("Menu"))
                .unwrap();
            let item = cx
                .create_detached_component(document, ActionMenuItem::new("Open"))
                .unwrap();
            cx.append_child(menu, item).unwrap();
            cx.append_child(root, menu).unwrap();
            product_frame(&mut cx, document, viewport(), &mut bundled_face_shaper());
            (cx, item.stable_id())
        };
        let (mut cx, item) = build(None);
        assert!(cx.world().layout_result(item).is_none());
        cx.set_theme_definition(&theme).unwrap();
        product_frame(&mut cx, document, viewport(), &mut bundled_face_shaper());
        assert!(cx.world().layout_result(item).is_none());
        let (mut cold, _) = build(Some(&theme));
        assert_matches_cold(&mut cx, &mut cold, document);
    }
}

mod menu_exit_withdrawal {
    //! Issue #387: a triggered menu that opens and then closes keeps its items
    //! laid out while its exit plays — the surface still shows, fading — and
    //! drops them from its trigger's overflow once the exit finishes, as a cold
    //! layout of the closed menu has none of them.

    use std::time::Duration;

    use crate::world::reflow_oracle::{assert_matches_cold, bundled_face_shaper, product_frame};
    use crate::{
        ActionMenu, ActionMenuItem, AppContext, DocumentId, Entity, LayoutViewport, Popover,
        StableNodeId, Stack,
    };
    use nana_ui_core::LengthSpec;

    fn viewport() -> LayoutViewport {
        LayoutViewport::new(700.0, 600.0)
    }

    /// A popover (`kind` 0) or an action menu (1) with one item, open or not,
    /// in a 420px row, laid out once.
    fn page(kind: u8, open: bool) -> (AppContext, StableNodeId) {
        let document = DocumentId::new(1).unwrap();
        let mut cx = AppContext::new();
        let root = cx
            .create_component(document, Stack::row(4.0).width(LengthSpec::Px(420.0)))
            .unwrap();
        let item = cx
            .create_detached_component(document, ActionMenuItem::new("Open"))
            .unwrap();
        let menu = if kind == 0 {
            let menu = cx
                .create_detached_component(document, Popover::new().trigger("Menu").open(open))
                .unwrap();
            cx.append_child(menu, item).unwrap();
            cx.append_child(root, menu).unwrap();
            menu.stable_id()
        } else {
            let mut view = ActionMenu::new();
            view.popover = view.popover.trigger("Menu").open(open);
            let menu = cx.create_detached_component(document, view).unwrap();
            cx.append_child(menu, item).unwrap();
            cx.append_child(root, menu).unwrap();
            menu.stable_id()
        };
        product_frame(&mut cx, document, viewport(), &mut bundled_face_shaper());
        (cx, menu)
    }

    fn set_open(cx: &mut AppContext, kind: u8, menu: StableNodeId, open: bool) {
        if kind == 0 {
            cx.update_component(Entity::<Popover>::from_stable_id(menu), |view, _| {
                view.open = open;
            })
            .unwrap();
        } else {
            cx.update_component(Entity::<ActionMenu>::from_stable_id(menu), |view, _| {
                view.popover.open = open;
            })
            .unwrap();
        }
    }

    /// Open, close, open, close. While the exit plays the items keep their
    /// boxes in the trigger's overflow; the frame after it finishes lays out as
    /// the closed menu laid out cold, the overflow the trigger's own box. Open
    /// steps match a cold open menu.
    #[test]
    fn issue387_a_closed_menu_drops_its_items_from_the_overflow_once_it_exits() {
        let document = DocumentId::new(1).unwrap();
        let mut shaper = bundled_face_shaper();
        for kind in 0..2u8 {
            let (mut cx, menu) = page(kind, false);
            let mut now = Duration::ZERO;
            for open in [true, false, true, false] {
                set_open(&mut cx, kind, menu, open);
                product_frame(&mut cx, document, viewport(), &mut shaper);
                if !open {
                    assert!(cx.world().surface_closing(menu), "kind {kind}");
                    let result = cx.world().layout_result(menu).unwrap();
                    assert_ne!(result.overflow, result.bounds, "kind {kind}");
                    now += Duration::from_secs(10);
                    cx.advance_animations(now);
                    product_frame(&mut cx, document, viewport(), &mut shaper);
                    assert!(!cx.world().surface_closing(menu), "kind {kind}");
                    let result = cx.world().layout_result(menu).unwrap();
                    assert_eq!(result.overflow, result.bounds, "kind {kind}");
                }
                let (mut cold, cold_menu) = page(kind, open);
                assert_eq!(cold_menu, menu);
                assert_matches_cold(&mut cx, &mut cold, document);
            }
        }
    }
}

mod hidden_context_reset {
    //! Issue #392: a container hidden after it was laid out keeps no layout
    //! context and leaves its descendants' boxes where a cold layout puts them,
    //! through hide, show and hide again.

    use nana_ui_core::{DisplaySpec, FlexDirection, LayoutStyle};

    use crate::world::reflow_oracle::{
        Builder, assert_matches_cold, bundled_face_shaper, product_frame, styled,
    };
    use crate::{
        AppContext, DocumentId, FormattingContextKind, LayoutBox, LayoutViewport, MutationQueue,
        StableNodeId,
    };

    fn viewport() -> LayoutViewport {
        LayoutViewport::new(600.0, 400.0)
    }

    fn container(hidden: bool) -> LayoutStyle {
        LayoutStyle {
            display: Some(DisplaySpec::Flex),
            direction: Some(FlexDirection::Column),
            hidden,
            ..LayoutStyle::default()
        }
    }

    /// A row holding a text and a flex column of two texts, one of them in a
    /// nested flex box, with the column hidden when asked, laid out once.
    /// Returns the column, the nested box and the text in it.
    fn page(hidden: bool) -> (AppContext, [StableNodeId; 3]) {
        let document = DocumentId::new(1).unwrap();
        let mut cx = AppContext::new();
        let (mut b, page) = Builder::page(document, 1, 400.0);
        let row = b.element(
            page,
            LayoutStyle {
                display: Some(DisplaySpec::Flex),
                direction: Some(FlexDirection::Row),
                ..LayoutStyle::default()
            },
        );
        b.label(row, "Before");
        let column = b.element(row, container(hidden));
        b.label(column, "First");
        let nested = b.element(column, container(false));
        let inner = b.label(nested, "Second");
        cx.commit_mutations(b.queue).unwrap();
        product_frame(&mut cx, document, viewport(), &mut bundled_face_shaper());
        (cx, [column, nested, inner])
    }

    /// Hidden, the column and the box in it establish no context and every box
    /// under it is the empty box at the origin; shown, they lay out again. Each
    /// step moves the column's context generation and counts no transition, and
    /// matches a cold layout of the same tree, boxes and contexts included.
    #[test]
    fn issue392_hiding_a_laid_out_container_matches_cold_layout() {
        let document = DocumentId::new(1).unwrap();
        let (mut cx, ids @ [column, nested, inner]) = page(false);
        for hidden in [true, false, true] {
            let mut q = MutationQueue::new();
            q.set_style(column, styled(container(hidden)));
            cx.commit_mutations(q).unwrap();
            let generation = cx.world().layout_node(column).unwrap().context_generation;
            let counters = product_frame(&mut cx, document, viewport(), &mut bundled_face_shaper());
            // Dropped or recorded again, a context is not switched for another.
            assert_eq!(counters.layout_context_transitions, 0, "hidden: {hidden}");
            let node = cx.world().layout_node(column).unwrap();
            assert_eq!(node.context_generation, generation + 1, "hidden: {hidden}");
            let established = |id| cx.world().layout_node(id).and_then(|node| node.established);
            let context = (!hidden).then_some(FormattingContextKind::Flex);
            assert_eq!(established(column), context, "hidden: {hidden}");
            assert_eq!(established(nested), context, "hidden: {hidden}");
            let collapsed = cx.world().layout_box(inner) == Some(LayoutBox::default());
            assert_eq!(collapsed, hidden);
            let (mut cold, cold_ids) = page(hidden);
            assert_eq!(cold_ids, ids);
            assert_matches_cold(&mut cx, &mut cold, document);
        }
    }
}

mod baseline_alignment {
    //! Issue #394: a flex container's baseline, read off where placement puts
    //! its items (`layout_engine::arrange_flow`) rather than laid out again.
    //!
    //! On a flex row aligning first baselines, a flex container sits by the
    //! line nearest the top of the page: by the baseline its items share under
    //! `align-self: baseline`, or else by its item nearest the inline start,
    //! wherever its alignment, its direction or `wrap-reverse` put that item.
    //! A column sits by its topmost item. A box holding no line box sits by its
    //! border-box bottom. Chrome's numbers for the same trees are the
    //! css-parity cases `T-FX01`..`T-FX03`.

    use nana_ui_core::{AlignSpec, DisplaySpec, FlexDirection, FlexWrap, JustifySpec, LayoutStyle};
    use nana_ui_core::{LengthSpec, LineHeightSpec};

    use crate::world::reflow_oracle::{
        Builder, assert_matches_cold, bundled_face_shaper, measured, product_frame, styled,
    };
    use crate::{AppContext, DocumentId, LayoutViewport, MutationQueue, StableNodeId};

    fn document() -> DocumentId {
        DocumentId::new(1).unwrap()
    }

    fn viewport() -> LayoutViewport {
        LayoutViewport::new(900.0, 900.0)
    }

    /// What the steps of the gate change.
    #[derive(Clone, Copy, Debug)]
    struct Page {
        /// The second shared item's label's font size.
        shared_font: f32,
        /// The centred container's first label's font size.
        centred_font: f32,
        /// How the centred container aligns its items.
        centred: AlignSpec,
    }

    const FIRST: Page = Page {
        shared_font: 16.0,
        centred_font: 16.0,
        centred: AlignSpec::Center,
    };

    /// The ids the gate reads.
    struct Ids {
        /// The row's 10×10 empty reference box, which sits by its bottom.
        reference: StableNodeId,
        /// Per container, the label it sits by and one it does not.
        pairs: Vec<(&'static str, StableNodeId, Option<StableNodeId>)>,
        shared_label: StableNodeId,
        centred_label: StableNodeId,
        centred: StableNodeId,
        /// A column beside the row the steps never touch.
        unrelated: StableNodeId,
    }

    fn flex(edit: impl FnOnce(&mut LayoutStyle)) -> LayoutStyle {
        let mut layout = LayoutStyle {
            display: Some(DisplaySpec::Flex),
            direction: Some(FlexDirection::Row),
            ..LayoutStyle::default()
        };
        edit(&mut layout);
        layout
    }

    fn sized(width: f32, height: Option<f32>) -> LayoutStyle {
        LayoutStyle {
            width: Some(LengthSpec::Px(width)),
            height: height.map(LengthSpec::Px),
            ..LayoutStyle::default()
        }
    }

    /// A flex row aligning first baselines: the reference, then
    /// - `shared`: two items sharing a baseline, under 7px of padding;
    /// - `centred`: two fixed boxes centred (or `page.centred`) in 100px;
    /// - `reversed`: `row-reverse`, centred, its leftmost item the second;
    /// - `column`: packed to its end, its topmost item the first;
    /// - `wrapped`: `wrap-reverse` into three lines, its top line the last;
    /// - `empty-first`: an empty box first, at the end of 100px.
    fn page(page: Page) -> (AppContext, Ids) {
        let (mut b, root) = Builder::page(document(), 1, 880.0);
        let row = b.element(
            root,
            flex(|layout| {
                layout.align_items = AlignSpec::Baseline;
                layout.width = Some(LengthSpec::Px(860.0));
            }),
        );
        let reference = b.element(row, sized(10.0, Some(10.0)));
        let font = |size: f32| LayoutStyle {
            font_size: Some(size),
            line_height: Some(LineHeightSpec::Relative(1.2)),
            ..LayoutStyle::default()
        };
        let labelled = |b: &mut Builder, parent, layout, text: &str, size| {
            let holder = b.element(parent, layout);
            let label = b.label(holder, text);
            b.queue.set_style(label, styled(font(size)));
            label
        };
        let mut pairs = Vec::new();

        let shared = b.element(
            row,
            flex(|layout| {
                layout.align_items = AlignSpec::Baseline;
                layout.padding_top = Some(LengthSpec::Px(7.0));
                layout.width = Some(LengthSpec::Px(130.0));
            }),
        );
        let first = labelled(&mut b, shared, sized(60.0, None), "P", 12.0);
        let shared_label = labelled(&mut b, shared, sized(60.0, None), "Q", page.shared_font);
        pairs.push(("shared", first, None));
        pairs.push(("shared", shared_label, None));

        let centred = b.element(
            row,
            flex(|layout| {
                layout.align_items = page.centred;
                layout.width = Some(LengthSpec::Px(130.0));
                layout.height = Some(LengthSpec::Px(100.0));
            }),
        );
        let centred_label = labelled(
            &mut b,
            centred,
            sized(60.0, Some(40.0)),
            "C",
            page.centred_font,
        );
        let other = labelled(&mut b, centred, sized(60.0, Some(60.0)), "D", 24.0);
        pairs.push(("centred", centred_label, Some(other)));

        let reversed = b.element(
            row,
            flex(|layout| {
                layout.flex_reverse = true;
                layout.align_items = AlignSpec::Center;
                layout.width = Some(LengthSpec::Px(130.0));
                layout.height = Some(LengthSpec::Px(100.0));
            }),
        );
        let right = labelled(&mut b, reversed, sized(60.0, Some(40.0)), "R", 16.0);
        let left = labelled(&mut b, reversed, sized(60.0, Some(60.0)), "L", 24.0);
        pairs.push(("reversed", left, Some(right)));

        let column = b.element(
            row,
            flex(|layout| {
                layout.direction = Some(FlexDirection::Column);
                layout.justify_content = JustifySpec::End;
                layout.width = Some(LengthSpec::Px(100.0));
                layout.height = Some(LengthSpec::Px(140.0));
            }),
        );
        let top = labelled(&mut b, column, sized(60.0, None), "T", 16.0);
        let below = labelled(&mut b, column, sized(60.0, None), "B", 24.0);
        pairs.push(("column", top, Some(below)));

        let wrapped = b.element(
            row,
            flex(|layout| {
                layout.flex_wrap = FlexWrap::WrapReverse;
                layout.align_items = AlignSpec::Baseline;
                layout.width = Some(LengthSpec::Px(100.0));
            }),
        );
        let first_line = labelled(&mut b, wrapped, sized(60.0, None), "W", 16.0);
        labelled(&mut b, wrapped, sized(60.0, None), "X", 16.0);
        let top_line = labelled(&mut b, wrapped, sized(60.0, None), "Y", 24.0);
        pairs.push(("wrapped", top_line, Some(first_line)));

        let empty_first = b.element(
            row,
            flex(|layout| {
                layout.align_items = AlignSpec::End;
                layout.width = Some(LengthSpec::Px(100.0));
                layout.height = Some(LengthSpec::Px(100.0));
            }),
        );
        let empty = b.element(empty_first, sized(30.0, Some(30.0)));
        let after = labelled(&mut b, empty_first, sized(60.0, None), "E", 16.0);
        pairs.push(("empty-first", empty, Some(after)));

        let unrelated = b.element(root, sized(400.0, None));
        for index in 0..20 {
            b.label(unrelated, &format!("Unrelated {index}"));
        }
        let mut context = AppContext::new();
        context.commit_mutations(b.queue).unwrap();
        product_frame(
            &mut context,
            document(),
            viewport(),
            &mut bundled_face_shaper(),
        );
        (
            context,
            Ids {
                reference,
                pairs,
                shared_label,
                centred_label,
                centred,
                unrelated,
            },
        )
    }

    /// Each container's sitting label (or empty box, by its bottom) is on the
    /// reference box's bottom, and its other one is not.
    fn assert_on_the_reference(context: &AppContext, ids: &Ids) {
        let world = context.world();
        let bottom = |id: StableNodeId| {
            let layout = world.layout_box(id).unwrap();
            layout.y + layout.height
        };
        let baseline = |id: StableNodeId| {
            if matches!(world.node(id).unwrap().kind, crate::world::NodeKind::Text) {
                world
                    .layout_result(id)
                    .and_then(|result| result.first_baseline)
                    .unwrap_or_else(|| panic!("{id:?} publishes no baseline"))
            } else {
                bottom(id)
            }
        };
        let reference = bottom(ids.reference);
        for &(name, sitting, other) in &ids.pairs {
            assert!(
                (baseline(sitting) - reference).abs() < 0.01,
                "{name}: {} vs the reference's bottom {reference}",
                baseline(sitting)
            );
            if let Some(other) = other {
                assert!(
                    (baseline(other) - reference).abs() > 1.0,
                    "{name}: the other item is on the baseline too"
                );
            }
        }
    }

    fn set_font(queue: &mut MutationQueue, label: StableNodeId, size: f32) {
        queue.set_style(
            label,
            styled(LayoutStyle {
                font_size: Some(size),
                line_height: Some(LineHeightSpec::Relative(1.2)),
                ..LayoutStyle::default()
            }),
        );
    }

    /// A flex container on a baseline-aligned row sits by its top line's shared
    /// baseline or by its start item's, where placement put them (#394). A
    /// bigger font inside a fixed centred item, a bigger font in a shared item
    /// and a new alignment each align the row again, as a cold layout does,
    /// and lay out nothing beside the row.
    #[test]
    fn issue394_a_flex_container_sits_by_its_top_lines_baseline() {
        let mut shaper = bundled_face_shaper();
        let (mut warm, ids) = page(FIRST);
        assert_on_the_reference(&warm, &ids);
        let (mut cold, _) = page(FIRST);
        assert_matches_cold(&mut warm, &mut cold, document());

        let steps = [
            Page {
                centred_font: 28.0,
                ..FIRST
            },
            Page {
                centred_font: 28.0,
                shared_font: 30.0,
                ..FIRST
            },
            Page {
                centred_font: 28.0,
                shared_font: 30.0,
                centred: AlignSpec::End,
            },
        ];
        let mut previous = FIRST;
        for step in steps {
            let items = |context: &AppContext| {
                let world = context.world();
                let row = world.parent_id(ids.reference).unwrap();
                world
                    .node(row)
                    .unwrap()
                    .children
                    .iter()
                    .map(|id| world.layout_box(*id))
                    .collect::<Vec<_>>()
            };
            let before = items(&warm);
            let mut queue = MutationQueue::new();
            if step.centred_font != previous.centred_font {
                set_font(&mut queue, ids.centred_label, step.centred_font);
            }
            if step.shared_font != previous.shared_font {
                set_font(&mut queue, ids.shared_label, step.shared_font);
            }
            if step.centred != previous.centred {
                queue.set_style(
                    ids.centred,
                    styled(flex(|layout| {
                        layout.align_items = step.centred;
                        layout.width = Some(LengthSpec::Px(130.0));
                        layout.height = Some(LengthSpec::Px(100.0));
                    })),
                );
            }
            warm.commit_mutations(queue).unwrap();
            product_frame(&mut warm, document(), viewport(), &mut shaper);
            assert_on_the_reference(&warm, &ids);
            assert_ne!(items(&warm), before, "{step:?}: the row aligned again");
            assert!(
                !measured(&warm, ids.unrelated),
                "{step:?}: the column beside the row laid out"
            );
            let (mut cold, _) = page(step);
            assert_matches_cold(&mut warm, &mut cold, document());
            previous = step;
        }
    }

    /// A flex row aligning first baselines: the empty reference box, a block
    /// flow 120 tall whose first child takes an auto top margin, and a block
    /// flow whose first label clears a float. Returns the reference and each
    /// flow's first label.
    fn block_flow_page() -> (AppContext, [StableNodeId; 3]) {
        let (mut b, root) = Builder::page(document(), 1, 880.0);
        let row = b.element(
            root,
            flex(|layout| {
                layout.align_items = AlignSpec::Baseline;
                layout.width = Some(LengthSpec::Px(860.0));
            }),
        );
        let reference = b.element(row, sized(10.0, Some(10.0)));
        let auto = b.element(row, sized(120.0, Some(120.0)));
        let pushed = b.element(
            auto,
            LayoutStyle {
                margin_top: Some(LengthSpec::Auto),
                ..LayoutStyle::default()
            },
        );
        let auto_label = b.label(pushed, "Auto");
        let floats = b.element(row, sized(120.0, None));
        b.element(
            floats,
            LayoutStyle {
                float: nana_ui_core::FloatSpec::Left,
                ..sized(40.0, Some(50.0))
            },
        );
        let cleared = b.element(
            floats,
            LayoutStyle {
                clear: nana_ui_core::ClearSpec::Both,
                ..LayoutStyle::default()
            },
        );
        let clear_label = b.label(cleared, "Clear");
        let mut context = AppContext::new();
        context.commit_mutations(b.queue).unwrap();
        product_frame(
            &mut context,
            document(),
            viewport(),
            &mut bundled_face_shaper(),
        );
        (context, [reference, auto_label, clear_label])
    }

    /// A block flow sits by its first line box where placement put it: below
    /// the space an auto margin takes, and below a float its block clears.
    #[test]
    fn issue394_a_block_flow_sits_by_where_placement_put_its_first_line() {
        let (context, [reference, auto_label, clear_label]) = block_flow_page();
        let world = context.world();
        let reference = world.layout_box(reference).unwrap();
        let reference = reference.y + reference.height;
        for label in [auto_label, clear_label] {
            let parent = world.layout_box(world.parent_id(label).unwrap()).unwrap();
            let flow = world.layout_box(world.parent_id(world.parent_id(label).unwrap()).unwrap());
            assert!(
                parent.y > flow.unwrap().y + 40.0,
                "{parent:?} is not pushed down in {flow:?}"
            );
            let baseline = world.layout_result(label).unwrap().first_baseline.unwrap();
            assert!(
                (baseline - reference).abs() < 0.01,
                "{label:?}: {baseline} vs the reference's bottom {reference}"
            );
        }
    }

    /// What the grid gate's steps change.
    #[derive(Clone, Copy, Debug)]
    struct GridPage {
        /// The font size of the label in the centred grid's fixed first item.
        centred_font: f32,
        /// The font size of the second row's first label in the baseline grid.
        second_row_font: f32,
    }

    const GRID_FIRST: GridPage = GridPage {
        centred_font: 16.0,
        second_row_font: 16.0,
    };

    /// The grid gate's ids.
    struct GridIds {
        reference: StableNodeId,
        /// The baseline grid's labels, by row.
        rows: [[StableNodeId; 3]; 2],
        /// Per centred or placed grid, the label it sits by and one it does
        /// not.
        pairs: Vec<(&'static str, StableNodeId, StableNodeId)>,
        centred_label: StableNodeId,
        unrelated: StableNodeId,
    }

    fn grid(edit: impl FnOnce(&mut LayoutStyle)) -> LayoutStyle {
        let mut layout = LayoutStyle {
            display: Some(DisplaySpec::Grid),
            ..LayoutStyle::default()
        };
        edit(&mut layout);
        layout
    }

    /// A flex row aligning first baselines: the reference, then
    /// - `baseline`: three 60px columns aligning items by baseline, two rows;
    /// - `centred`: two fixed items centred in a 100px row;
    /// - `placed`: the second item placed in the first column, both centred.
    fn grid_page(page: GridPage) -> (AppContext, GridIds) {
        use nana_ui_core::{GridLine, GridPlacement, GridTrack};
        let (mut b, root) = Builder::page(document(), 1, 880.0);
        let row = b.element(
            root,
            flex(|layout| {
                layout.align_items = AlignSpec::Baseline;
                layout.width = Some(LengthSpec::Px(860.0));
            }),
        );
        let reference = b.element(row, sized(10.0, Some(10.0)));
        let font = |size: f32| LayoutStyle {
            font_size: Some(size),
            line_height: Some(LineHeightSpec::Relative(1.2)),
            ..LayoutStyle::default()
        };
        let labelled = |b: &mut Builder, parent, layout, text: &str, size| {
            let holder = b.element(parent, layout);
            let label = b.label(holder, text);
            b.queue.set_style(label, styled(font(size)));
            label
        };
        let columns = |count: usize| Some(vec![GridTrack::Px(60.0); count]);

        let baseline = b.element(
            row,
            grid(|layout| {
                layout.grid_columns = columns(3);
                layout.align_items = AlignSpec::Baseline;
                layout.width = Some(LengthSpec::Px(200.0));
            }),
        );
        let mut rows = [[reference; 3]; 2];
        for (index, (text, size)) in [
            ("A", 12.0),
            ("B", 24.0),
            ("C", 16.0),
            ("D", page.second_row_font),
            ("E", 12.0),
            ("F", 28.0),
        ]
        .into_iter()
        .enumerate()
        {
            rows[index / 3][index % 3] =
                labelled(&mut b, baseline, LayoutStyle::default(), text, size);
        }

        let mut pairs = Vec::new();
        let centred = b.element(
            row,
            grid(|layout| {
                layout.grid_columns = columns(2);
                layout.grid_rows = Some(vec![GridTrack::Px(100.0)]);
                layout.align_items = AlignSpec::Center;
                layout.width = Some(LengthSpec::Px(130.0));
            }),
        );
        let centred_label = labelled(
            &mut b,
            centred,
            sized(60.0, Some(40.0)),
            "G",
            page.centred_font,
        );
        let other = labelled(&mut b, centred, sized(60.0, Some(60.0)), "H", 24.0);
        pairs.push(("centred", centred_label, other));

        let placed = b.element(
            row,
            grid(|layout| {
                layout.grid_columns = columns(2);
                layout.grid_rows = Some(vec![GridTrack::Px(100.0)]);
                layout.align_items = AlignSpec::Center;
                layout.width = Some(LengthSpec::Px(130.0));
            }),
        );
        let at = |column: i32| LayoutStyle {
            grid_placement: GridPlacement {
                column_start: GridLine::Index(column),
                row_start: GridLine::Index(1),
                ..GridPlacement::default()
            }
            .into(),
            ..sized(60.0, None)
        };
        let second_column = labelled(&mut b, placed, at(2), "I", 16.0);
        let first_column = labelled(&mut b, placed, at(1), "J", 24.0);
        pairs.push(("placed", first_column, second_column));

        let unrelated = b.element(root, sized(400.0, None));
        for index in 0..20 {
            b.label(unrelated, &format!("Unrelated {index}"));
        }
        let mut context = AppContext::new();
        context.commit_mutations(b.queue).unwrap();
        product_frame(
            &mut context,
            document(),
            viewport(),
            &mut bundled_face_shaper(),
        );
        (
            context,
            GridIds {
                reference,
                rows,
                pairs,
                centred_label,
                unrelated,
            },
        )
    }

    /// Each row of the baseline grid shares one baseline, its first the
    /// reference's bottom; each other grid's sitting label is on it and its
    /// other label is not.
    fn assert_grids_on_the_reference(context: &AppContext, ids: &GridIds) {
        let world = context.world();
        let baseline = |id: StableNodeId| {
            world
                .layout_result(id)
                .and_then(|result| result.first_baseline)
                .unwrap_or_else(|| panic!("{id:?} publishes no baseline"))
        };
        let reference = world.layout_box(ids.reference).unwrap();
        let reference = reference.y + reference.height;
        for (index, row) in ids.rows.iter().enumerate() {
            let shared = baseline(row[0]);
            for label in row {
                assert!(
                    (baseline(*label) - shared).abs() < 0.01,
                    "row {index}: {} vs {shared}",
                    baseline(*label)
                );
            }
            assert_eq!(
                (shared - reference).abs() < 0.01,
                index == 0,
                "row {index} at {shared}, the reference's bottom at {reference}"
            );
        }
        for &(name, sitting, other) in &ids.pairs {
            assert!(
                (baseline(sitting) - reference).abs() < 0.01,
                "{name}: {} vs the reference's bottom {reference}",
                baseline(sitting)
            );
            assert!(
                (baseline(other) - reference).abs() > 1.0,
                "{name}: the other item is on the baseline too"
            );
        }
    }

    /// Grid items aligned by baseline share one per start row, and a grid on a
    /// baseline-aligned row sits by its first row's shared baseline or by its
    /// first item in grid order (#394). A bigger font in a centred fixed item
    /// aligns the outer row again; one in the second row moves that row's
    /// items and not the first. Each step matches a cold layout and lays out
    /// nothing beside the row.
    #[test]
    fn issue394_grid_items_share_a_baseline_per_row_and_a_grid_sits_by_its_first() {
        let mut shaper = bundled_face_shaper();
        let (mut warm, ids) = grid_page(GRID_FIRST);
        assert_grids_on_the_reference(&warm, &ids);
        let (mut cold, _) = grid_page(GRID_FIRST);
        assert_matches_cold(&mut warm, &mut cold, document());
        let first_row =
            |context: &AppContext| ids.rows[0].map(|label| context.world().layout_box(label));
        let mut previous = GRID_FIRST;
        for step in [
            GridPage {
                centred_font: 28.0,
                ..GRID_FIRST
            },
            GridPage {
                centred_font: 28.0,
                second_row_font: 32.0,
            },
        ] {
            let mut queue = MutationQueue::new();
            let moved = if step.centred_font != previous.centred_font {
                set_font(&mut queue, ids.centred_label, step.centred_font);
                ids.pairs[0].1
            } else {
                set_font(&mut queue, ids.rows[1][0], step.second_row_font);
                ids.rows[1][1]
            };
            let before = (warm.world().layout_box(moved), first_row(&warm));
            warm.commit_mutations(queue).unwrap();
            product_frame(&mut warm, document(), viewport(), &mut shaper);
            assert_grids_on_the_reference(&warm, &ids);
            let after = (warm.world().layout_box(moved), first_row(&warm));
            assert_ne!(after.0, before.0, "{step:?}: nothing aligned again");
            if step.second_row_font != previous.second_row_font {
                assert_eq!(after.1, before.1, "{step:?}: the first row moved");
            }
            assert!(
                !measured(&warm, ids.unrelated),
                "{step:?}: the column beside the row laid out"
            );
            let (mut cold, _) = grid_page(step);
            assert_matches_cold(&mut warm, &mut cold, document());
            previous = step;
        }
    }
}
