//! Scroll is a behavior of the same retained formatting context.
use super::reflow_oracle::{bundled_face_shaper, product_frame};
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
                .create_detached_component(document, Button::new("Item").layout(Arc::new(layout)))
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
        .project_accessibility(document)
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
    use super::super::reflow_oracle::{
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
            super::super::reflow_oracle::styled(LayoutStyle {
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
            .project_accessibility(document())
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
                super::super::reflow_oracle::styled(card(position)),
            );
            q.set_style(
                ids.inner,
                super::super::reflow_oracle::styled(scroller(200.0, 100.0, inner)),
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
                (PositionSpec::Relative, OverflowSpec::Auto) => (Some(ids.outer), Some(ids.card)),
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
                super::super::reflow_oracle::styled(card(position)),
            );
            q.set_style(
                ids.inner,
                super::super::reflow_oracle::styled(scroller(200.0, 100.0, inner)),
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
            super::super::reflow_oracle::styled(shifted(20.0)),
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
            super::super::reflow_oracle::styled(shifted(60.0)),
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
            super::super::reflow_oracle::styled(LayoutStyle {
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
            super::super::reflow_oracle::styled(LayoutStyle {
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
            super::super::reflow_oracle::styled(fixed(300.0, 30.0)),
        );
        context.commit_mutations(q).unwrap();
        hosted_frame(&mut context);
        assert_eq!(
            context.world().hit_test(document(), 215.0, 75.0),
            Some(ids.escaping)
        );
    }
}
