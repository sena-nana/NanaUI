//! A scroll view's edge fade as the scene hands it to a painter.
//!
//! The view isolates its subtree into one destination group that carries
//! the fade bands for the current offset. Its bars composite outside the
//! fade, and a view without one stays out of any group.

use std::time::Duration;

use nana_ui_core::{ColorFilter, LengthSpec, OverlayMount, PositionSpec, ScrollbarVisibility};
use nana_ui_runtime::view::{button, entity_ref, row, text, widget, with_refs};
use nana_ui_runtime::{
    ActionMenu, ActionMenuItem, ComputedStyle, ContextMenu, ContextMenuItem, Dialog, DocumentId,
    Entity, HoverCard, LayoutViewport, OverlayHost, Popover, ScrollAxes, ScrollOffset, ScrollView,
    Select, SelectOption, StableNodeId, Stack, TextContent, TextMetrics, TextShapeConstraints,
    TextShaper, Tooltip,
};
use nana_ui_scene::{RuntimeDocument, SceneRect};

struct NoText;

impl TextShaper for NoText {
    fn shape(
        &mut self,
        _id: StableNodeId,
        _text: &TextContent,
        _style: &ComputedStyle,
        _constraints: TextShapeConstraints,
    ) -> TextMetrics {
        TextMetrics::default()
    }
}

const TRIGGERED_SURFACE_SLOT: u64 = 0xFFFF_FFE0 << 32;

fn flush(runtime: &mut RuntimeDocument) {
    runtime
        .flush(LayoutViewport::new(800.0, 600.0), &mut NoText)
        .unwrap();
}

/// A 300×200 vertical scroll view holding `rows` rows 100 tall.
fn scroll_view(
    fade: Option<(f32, f32)>,
    rows: usize,
) -> (RuntimeDocument, Entity<ScrollView>, Vec<StableNodeId>) {
    let document = DocumentId::new(1).unwrap();
    let mut runtime = RuntimeDocument::new(document);
    let cx = runtime.context_mut();
    let mut view = ScrollView::new(ScrollAxes::Vertical)
        .scrollbars(ScrollbarVisibility::Always)
        .with_layout(|layout| {
            layout.width = Some(LengthSpec::Px(300.0));
            layout.height = Some(LengthSpec::Px(200.0));
        });
    if let Some((top, bottom)) = fade {
        view = view.edge_fade(top, bottom);
    }
    let scroll = cx.create_component(document, view).unwrap();
    let column = cx.create_component(document, Stack::column(0.0)).unwrap();
    cx.append_child(scroll, column).unwrap();
    let mut ids = Vec::new();
    for _ in 0..rows {
        let row = cx
            .create_component(document, Stack::column(0.0).height(LengthSpec::Px(100.0)))
            .unwrap();
        cx.append_child(column, row).unwrap();
        ids.push(row.stable_id());
    }
    flush(&mut runtime);
    (runtime, scroll, ids)
}

fn scroll_to(runtime: &mut RuntimeDocument, scroll: Entity<ScrollView>, y: f32) {
    runtime
        .context_mut()
        .scroll_to(scroll, ScrollOffset { x: 0.0, y })
        .unwrap();
    flush(runtime);
}

/// The bands the row's group fades with, `[top, bottom]`.
fn bands(runtime: &RuntimeDocument, scroll: Entity<ScrollView>, row: StableNodeId) -> [f32; 2] {
    let groups = runtime.scene().opacity_groups(row);
    let [group] = groups.as_ref() else {
        panic!("one group around the scrolled content: {groups:?}");
    };
    assert_eq!(group.node, scroll.stable_id());
    let fade = group.edge_fade.expect("the group fades its edges");
    [fade.top, fade.bottom]
}

#[test]
fn the_bands_follow_the_content_past_each_edge() {
    let (mut runtime, scroll, rows) = scroll_view(Some((14.0, 64.0)), 10);
    let row = rows[0];
    // At rest nothing is above: only the bottom band shows.
    assert_eq!(bands(&runtime, scroll, row), [0.0, 64.0]);
    let fade = runtime.scene().opacity_groups(row)[0].edge_fade.unwrap();
    let port = runtime.scene().node_bounds(scroll.stable_id()).unwrap();
    // Only the vertical axis clips, so the clip box runs on past the sides.
    assert_eq!(
        (fade.bounds.y, fade.bounds.height),
        (port.y, port.height),
        "the bands run from the scrollport's edges"
    );
    // The top band ramps in with the offset rather than popping.
    scroll_to(&mut runtime, scroll, 5.0);
    assert_eq!(bands(&runtime, scroll, row), [5.0, 64.0]);
    scroll_to(&mut runtime, scroll, 400.0);
    assert_eq!(bands(&runtime, scroll, row), [14.0, 64.0]);
    // 1000 of content in a 200 port: 20 left below shortens the bottom band.
    scroll_to(&mut runtime, scroll, 780.0);
    assert_eq!(bands(&runtime, scroll, row), [14.0, 20.0]);
    scroll_to(&mut runtime, scroll, 800.0);
    assert_eq!(bands(&runtime, scroll, row), [14.0, 0.0]);
    let groups = runtime.scene().opacity_groups(row);
    assert!(!groups[0].is_identity());
    assert!(groups[0].is_edge_fade_only());
}

#[test]
fn content_that_fits_keeps_the_group_but_fades_nothing() {
    let (runtime, scroll, rows) = scroll_view(Some((14.0, 64.0)), 1);
    assert_eq!(bands(&runtime, scroll, rows[0]), [0.0, 0.0]);
    assert!(
        runtime.scene().opacity_groups(rows[0])[0].is_identity(),
        "a painter composites nothing while neither band shows"
    );
}

#[test]
fn the_bottom_band_follows_content_that_grows_and_shrinks() {
    let document = DocumentId::new(1).unwrap();
    let mut runtime = RuntimeDocument::new(document);
    let cx = runtime.context_mut();
    let scroll = cx
        .create_component(
            document,
            ScrollView::new(ScrollAxes::Vertical)
                .edge_fade(14.0, 64.0)
                .with_layout(|layout| {
                    layout.width = Some(LengthSpec::Px(300.0));
                    layout.height = Some(LengthSpec::Px(200.0));
                }),
        )
        .unwrap();
    let column = cx.create_component(document, Stack::column(0.0)).unwrap();
    cx.append_child(scroll, column).unwrap();
    let first = cx
        .create_component(document, Stack::column(0.0).height(LengthSpec::Px(150.0)))
        .unwrap();
    cx.append_child(column, first).unwrap();
    flush(&mut runtime);
    let row = first.stable_id();
    assert_eq!(bands(&runtime, scroll, row), [0.0, 0.0]);
    let mut added = Vec::new();
    for _ in 0..3 {
        let cx = runtime.context_mut();
        let more = cx
            .create_component(document, Stack::column(0.0).height(LengthSpec::Px(30.0)))
            .unwrap();
        cx.append_child(column, more).unwrap();
        added.push(more);
        flush(&mut runtime);
    }
    // 240 of content in a 200 port: 40 left below shortens the band.
    assert_eq!(bands(&runtime, scroll, row), [0.0, 40.0]);
    scroll_to(&mut runtime, scroll, 30.0);
    assert_eq!(bands(&runtime, scroll, row), [14.0, 10.0]);
    for more in added {
        runtime.context_mut().remove_view(more).unwrap();
        flush(&mut runtime);
    }
    // The offset is clamped back to 0 and nothing overflows any more.
    assert_eq!(bands(&runtime, scroll, row), [0.0, 0.0]);
}

#[test]
fn only_a_view_with_a_fade_is_isolated() {
    let (runtime, _, rows) = scroll_view(None, 10);
    assert!(runtime.scene().opacity_groups(rows[0]).is_empty());
    let (runtime, _, rows) = scroll_view(Some((0.0, 0.0)), 10);
    assert!(runtime.scene().opacity_groups(rows[0]).is_empty());
}

#[test]
fn the_bars_composite_outside_the_fade() {
    let (mut runtime, scroll, _) = scroll_view(Some((14.0, 64.0)), 10);
    scroll_to(&mut runtime, scroll, 400.0);
    let scene = runtime.scene();
    let node = scroll.stable_id();
    let own = scene.opacity_groups(node);
    assert!(own.last().is_some_and(|group| group.edge_fade.is_some()));
    // Scrollbar chrome takes the top half of the slot space.
    let bars: Vec<_> = scene
        .primitives()
        .filter(|primitive| primitive.node == node && primitive.id.slot >> 32 == 0xFFFF_FFF8)
        .collect();
    assert!(!bars.is_empty(), "the bars are drawn");
    for bar in bars {
        assert!(
            scene.primitive_opacity_groups(bar).is_empty(),
            "{:?} stays unfaded",
            bar.id
        );
    }
}

#[test]
fn rows_scrolled_out_of_the_port_are_still_culled() {
    let (mut runtime, scroll, rows) = scroll_view(Some((14.0, 64.0)), 40);
    scroll_to(&mut runtime, scroll, 1500.0);
    let scene = runtime.scene();
    let drawn: Vec<_> = scene
        .visible_operations(SceneRect {
            x: 0.0,
            y: 0.0,
            width: 800.0,
            height: 600.0,
        })
        .unwrap()
        .iter()
        .filter_map(|operation| match operation {
            nana_ui_scene::RenderOperation::Draw(id) => Some(id.node),
            _ => None,
        })
        .collect();
    assert!(
        !drawn.contains(&rows[0]),
        "a row far above the port is culled"
    );
    assert!(
        !drawn.contains(&rows[39]),
        "a row far below the port is culled"
    );
}

#[test]
fn a_triggered_popover_escapes_the_scrollports_edge_fade() {
    const TRIGGERED_SURFACE_SLOT: u64 = 0xFFFF_FFE0 << 32;

    let setup = |outer_opacity: Option<f32>, advance_ms: u64| {
        let document = DocumentId::new(1).unwrap();
        let mut runtime = RuntimeDocument::new(document);
        let root = runtime
            .context_mut()
            .create_component(
                document,
                Stack::column(0.0).with_layout(|layout| layout.opacity = outer_opacity),
            )
            .unwrap();
        let (_, (outer_scroll, popover, mark, item, inner_scroll, inner_row)) = runtime
            .context_mut()
            .mount_view(root.stable_id(), || {
                let outer_scroll = entity_ref::<ScrollView>();
                let popover = entity_ref::<Popover>();
                let mark = entity_ref::<nana_ui_runtime::Text>();
                let item = entity_ref::<nana_ui_runtime::Button>();
                let inner_scroll = entity_ref::<ScrollView>();
                let inner_row = entity_ref::<Stack>();
                let view = widget(
                    ScrollView::new(ScrollAxes::Vertical)
                        .edge_fade(14.0, 64.0)
                        .with_layout(|layout| {
                            layout.width = Some(LengthSpec::Px(300.0));
                            layout.height = Some(LengthSpec::Px(200.0));
                        }),
                )
                .entity_ref(outer_scroll)
                .children(
                    widget(Stack::column(0.0).height(LengthSpec::Px(400.0))).children(
                        widget(Popover::new().trigger("Open"))
                            .entity_ref(popover)
                            .trigger(row().children(text("Mark").entity_ref(mark)))
                            .children((
                                button("Action").entity_ref(item),
                                widget(
                                    ScrollView::new(ScrollAxes::Vertical)
                                        .edge_fade(10.0, 20.0)
                                        .with_layout(|layout| {
                                            layout.width = Some(LengthSpec::Px(180.0));
                                            layout.height = Some(LengthSpec::Px(80.0));
                                        }),
                                )
                                .entity_ref(inner_scroll)
                                .children(
                                    widget(Stack::column(0.0).height(LengthSpec::Px(200.0)))
                                        .children(
                                            widget(Stack::column(0.0).height(LengthSpec::Px(40.0)))
                                                .entity_ref(inner_row),
                                        ),
                                ),
                            )),
                    ),
                );
                with_refs(
                    view,
                    (outer_scroll, popover, mark, item, inner_scroll, inner_row),
                )
            })
            .unwrap();
        runtime
            .context_mut()
            .advance_animations(Duration::from_millis(1000));
        flush(&mut runtime);
        runtime
            .context_mut()
            .update_component(popover, |popover, _| popover.open = true)
            .unwrap();
        runtime
            .context_mut()
            .advance_animations(Duration::from_millis(advance_ms));
        flush(&mut runtime);
        (
            runtime,
            root,
            outer_scroll,
            popover,
            mark,
            item,
            inner_scroll,
            inner_row,
        )
    };

    let (runtime, root, outer_scroll, popover, mark, item, inner_scroll, inner_row) =
        setup(Some(0.5), 1030);
    let scene = runtime.scene();
    let root_id = root.stable_id();
    let outer_id = outer_scroll.stable_id();
    let popover_id = popover.stable_id();
    let inner_id = inner_scroll.stable_id();

    let trigger_groups = scene.primitive_opacity_groups(
        scene
            .primitives()
            .find(|p| p.node == mark.stable_id())
            .expect("trigger text primitive"),
    );
    assert!(
        trigger_groups
            .iter()
            .any(|group| group.node == outer_id && group.edge_fade.is_some()),
        "trigger stays in the outer scroll view's edge fade: {trigger_groups:?}"
    );

    let surface_groups = scene.primitive_opacity_groups(
        scene
            .primitives()
            .find(|p| p.node == popover_id && p.id.slot == TRIGGERED_SURFACE_SLOT)
            .expect("popover surface primitive"),
    );
    let item_groups = scene.primitive_opacity_groups(
        scene
            .primitives()
            .find(|p| p.node == item.stable_id())
            .expect("popover button primitive"),
    );
    assert_eq!(
        surface_groups.as_ref(),
        item_groups.as_ref(),
        "mid-fade surface and fixed content share the exact same group list"
    );
    assert!(
        surface_groups.iter().all(|group| group.node != root_id
            && group.node != outer_id
            && group.edge_fade.is_none()),
        "isolated popover strips outer ancestor opacity and scroll edge fade mid-fade: {surface_groups:?}"
    );
    assert!(
        surface_groups
            .iter()
            .any(|group| group.node == popover_id && group.opacity > 0.0 && group.opacity < 1.0),
        "isolated popover retains its own open/close fade: {surface_groups:?}"
    );
    let _ = root_id;

    let inner_row_groups = scene.opacity_groups(inner_row.stable_id());
    assert!(
        inner_row_groups.iter().all(|group| group.node != outer_id),
        "nested scroll view inside popover drops outer edge fade: {inner_row_groups:?}"
    );
    assert!(
        inner_row_groups
            .iter()
            .any(|group| group.node == inner_id && group.edge_fade.is_some()),
        "nested scroll view inside popover still fades its own rows: {inner_row_groups:?}"
    );

    // Once opening settles and the outer ancestor is opaque, floating groups are empty.
    let (settled_runtime, _, _, settled_popover, _, settled_item, _, _) = setup(None, 2000);
    let settled_scene = settled_runtime.scene();
    for primitive in settled_scene.primitives() {
        if (primitive.node == settled_popover.stable_id()
            && primitive.id.slot == TRIGGERED_SURFACE_SLOT)
            || primitive.node == settled_item.stable_id()
        {
            assert!(
                settled_scene.primitive_opacity_groups(primitive).is_empty(),
                "settled popover {:?} has no leftover edge fade group",
                primitive.id
            );
        }
    }
}

#[test]
fn an_open_select_menu_escapes_the_scrollports_edge_fade() {
    let document = DocumentId::new(1).unwrap();
    let mut runtime = RuntimeDocument::new(document);
    let cx = runtime.context_mut();
    let scroll = cx
        .create_component(
            document,
            ScrollView::new(ScrollAxes::Vertical)
                .edge_fade(14.0, 64.0)
                .with_layout(|layout| {
                    layout.width = Some(LengthSpec::Px(300.0));
                    layout.height = Some(LengthSpec::Px(200.0));
                }),
        )
        .unwrap();
    let column = cx
        .create_component(document, Stack::column(0.0).height(LengthSpec::Px(400.0)))
        .unwrap();
    cx.append_child(scroll, column).unwrap();
    let mut select = Select::new(Some("a"));
    select.options = vec![
        SelectOption::new("a", "Alpha"),
        SelectOption::new("b", "Beta"),
    ];
    let select = cx.create_component(document, select).unwrap();
    cx.append_child(column, select).unwrap();
    flush(&mut runtime);

    runtime.context_mut().toggle_select(select).unwrap();
    flush(&mut runtime);

    let scene = runtime.scene();
    let select_id = select.stable_id();
    let scroll_id = scroll.stable_id();
    let mut saw_field = false;
    let mut saw_menu = false;
    for primitive in scene.primitives() {
        if primitive.node != select_id {
            continue;
        }
        let slot_ns = primitive.id.slot >> 32;
        let is_menu =
            primitive.id.slot == 8 || (9..=13).contains(&slot_ns) || (17..=18).contains(&slot_ns);
        let groups = scene.primitive_opacity_groups(primitive);
        if is_menu {
            saw_menu = true;
            assert!(
                groups
                    .iter()
                    .all(|group| group.node != scroll_id && group.edge_fade.is_none()),
                "open select menu primitive {:?} escapes scroll edge fade: {groups:?}",
                primitive.id
            );
        } else if primitive.id.slot == 0 {
            saw_field = true;
            assert!(
                groups
                    .iter()
                    .any(|group| group.node == scroll_id && group.edge_fade.is_some()),
                "select field stays inside scroll edge fade: {groups:?}"
            );
        }
    }
    assert!(saw_field && saw_menu, "both select field and menu painted");
}

#[test]
fn an_escaping_absolute_box_drops_only_bypassed_edge_fades() {
    let document = DocumentId::new(1).unwrap();
    let mut runtime = RuntimeDocument::new(document);
    let cx = runtime.context_mut();
    let outer_container = cx
        .create_component(
            document,
            Stack::column(0.0).with_layout(|layout| layout.position = PositionSpec::Relative),
        )
        .unwrap();
    let outer_scroll = cx
        .create_component(
            document,
            ScrollView::new(ScrollAxes::Vertical)
                .edge_fade(14.0, 64.0)
                .with_layout(|layout| {
                    layout.position = PositionSpec::Relative;
                    layout.width = Some(LengthSpec::Px(300.0));
                    layout.height = Some(LengthSpec::Px(200.0));
                }),
        )
        .unwrap();
    cx.append_child(outer_container, outer_scroll).unwrap();
    let inner_scroll = cx
        .create_component(
            document,
            ScrollView::new(ScrollAxes::Vertical)
                .edge_fade(10.0, 30.0)
                .with_layout(|layout| {
                    layout.width = Some(LengthSpec::Px(280.0));
                    layout.height = Some(LengthSpec::Px(400.0));
                }),
        )
        .unwrap();
    cx.append_child(outer_scroll, inner_scroll).unwrap();
    let in_flow = cx
        .create_component(document, Stack::column(0.0).height(LengthSpec::Px(600.0)))
        .unwrap();
    cx.append_child(inner_scroll, in_flow).unwrap();
    let escaping = cx
        .create_component(
            document,
            Stack::column(0.0).with_layout(|layout| {
                layout.position = PositionSpec::Absolute;
                layout.width = Some(LengthSpec::Px(80.0));
                layout.height = Some(LengthSpec::Px(80.0));
            }),
        )
        .unwrap();
    cx.append_child(inner_scroll, escaping).unwrap();
    flush(&mut runtime);

    let scene = runtime.scene();
    let outer_id = outer_scroll.stable_id();
    let inner_id = inner_scroll.stable_id();
    let in_flow_groups = scene.opacity_groups(in_flow.stable_id());
    assert!(
        in_flow_groups
            .iter()
            .any(|g| g.node == outer_id && g.edge_fade.is_some())
            && in_flow_groups
                .iter()
                .any(|g| g.node == inner_id && g.edge_fade.is_some()),
        "in-flow row keeps both edge fades: {in_flow_groups:?}"
    );
    let escaping_groups = scene.opacity_groups(escaping.stable_id());
    assert!(
        escaping_groups
            .iter()
            .any(|g| g.node == outer_id && g.edge_fade.is_some()),
        "escaping absolute box keeps outer clipping scroll edge fade: {escaping_groups:?}"
    );
    assert!(
        escaping_groups.iter().all(|g| g.node != inner_id),
        "escaping absolute box drops bypassed inner scroll edge fade: {escaping_groups:?}"
    );
}

#[test]
fn isolated_menus_and_overlays_do_not_inherit_ancestor_effects() {
    let document = DocumentId::new(1).unwrap();
    let mut runtime = RuntimeDocument::new(document);
    let root = runtime
        .context_mut()
        .create_component(
            document,
            Stack::column(0.0).with_layout(|layout| {
                layout.opacity = Some(0.45);
                layout.paint.filter = Some(ColorFilter {
                    blur_radius: 4.0,
                    ..ColorFilter::default()
                });
                layout.width = Some(LengthSpec::Px(800.0));
                layout.height = Some(LengthSpec::Px(600.0));
            }),
        )
        .unwrap();

    let (
        _,
        (
            (scroll, pop, pop_mark, pop_item),
            (menu, menu_mark, menu_item),
            (card, card_item, select, ctx, tip),
        ),
    ) = runtime
        .context_mut()
        .mount_view(root.stable_id(), || {
            let scroll = entity_ref::<ScrollView>();
            let pop = entity_ref::<Popover>();
            let pop_mark = entity_ref::<Stack>();
            let pop_item = entity_ref::<nana_ui_runtime::Button>();
            let menu = entity_ref::<ActionMenu>();
            let menu_mark = entity_ref::<Stack>();
            let menu_item = entity_ref::<ActionMenuItem>();
            let card = entity_ref::<HoverCard>();
            let card_item = entity_ref::<nana_ui_runtime::Button>();
            let select = entity_ref::<Select>();
            let ctx = entity_ref::<ContextMenu>();
            let tip = entity_ref::<Tooltip>();

            let tree = widget(
                ScrollView::new(ScrollAxes::Vertical)
                    .edge_fade(14.0, 64.0)
                    .with_layout(|layout| {
                        layout.width = Some(LengthSpec::Px(420.0));
                        layout.height = Some(LengthSpec::Px(260.0));
                    }),
            )
            .entity_ref(scroll)
            .children((
                widget(Stack::column(0.0).height(LengthSpec::Px(140.0))),
                widget(Popover::new().trigger("Pop").width(140.0))
                    .entity_ref(pop)
                    .trigger(
                        widget(Stack::row(0.0).height(LengthSpec::Px(24.0)))
                            .entity_ref(pop_mark)
                            .children(text("Pop")),
                    )
                    .children(button("Item").entity_ref(pop_item)),
                widget(ActionMenu::new().trigger("Menu"))
                    .entity_ref(menu)
                    .trigger(
                        widget(Stack::row(0.0).height(LengthSpec::Px(24.0)))
                            .entity_ref(menu_mark)
                            .children(text("Menu")),
                    )
                    .children(widget(ActionMenuItem::new("One")).entity_ref(menu_item)),
                widget(HoverCard::new().trigger("Card").width(140.0))
                    .entity_ref(card)
                    .children(button("Details").entity_ref(card_item)),
                widget(Select::new(Some("x")).options(vec![
                    SelectOption::new("x", "X"),
                    SelectOption::new("y", "Y"),
                ]))
                .entity_ref(select),
                widget(
                    ContextMenu::new(120.0, 120.0).items(vec![ContextMenuItem::new("c", "Copy")]),
                )
                .entity_ref(ctx),
                widget(Tooltip::new("Hint")).entity_ref(tip),
                widget(Stack::column(0.0).height(LengthSpec::Px(500.0))),
            ));
            with_refs(
                tree,
                (
                    (scroll, pop, pop_mark, pop_item),
                    (menu, menu_mark, menu_item),
                    (card, card_item, select, ctx, tip),
                ),
            )
        })
        .unwrap();

    runtime
        .context_mut()
        .advance_animations(Duration::from_millis(1000));
    flush(&mut runtime);
    let cx = runtime.context_mut();
    cx.update_component(pop, |p, _| p.open = true).unwrap();
    cx.update_component(menu, |m, _| m.popover.open = true)
        .unwrap();
    cx.update_component(card, |c, _| c.open = true).unwrap();
    cx.toggle_select(select).unwrap();
    // Advance mid-fade (30 ms past 1000 ms) to verify open fade stays intact without inheriting outer ancestors.
    cx.advance_animations(Duration::from_millis(1030));
    flush(&mut runtime);

    let scene = runtime.scene();
    let root_id = root.stable_id();
    let scroll_id = scroll.stable_id();

    for trigger in [pop_mark.stable_id(), menu_mark.stable_id()] {
        let groups = scene.opacity_groups(trigger);
        assert!(
            groups
                .iter()
                .any(|g| g.node == root_id && (g.opacity - 0.45).abs() < 1e-4)
                && groups
                    .iter()
                    .any(|g| g.node == scroll_id && g.edge_fade.is_some()),
            "in-flow trigger {trigger:?} keeps ancestor opacity and scroll edge fade: {groups:?}"
        );
        assert!(
            scene
                .filter_groups(trigger)
                .iter()
                .any(|g| g.node == root_id),
            "in-flow trigger {trigger:?} keeps ancestor filter group"
        );
    }
    let card_trigger_prim = scene
        .primitives()
        .find(|p| p.node == card.stable_id() && p.id.slot != TRIGGERED_SURFACE_SLOT)
        .expect("hover card trigger primitive");
    let card_trigger_groups = scene.primitive_opacity_groups(card_trigger_prim);
    assert!(
        card_trigger_groups
            .iter()
            .any(|g| g.node == root_id && (g.opacity - 0.45).abs() < 1e-4)
            && card_trigger_groups
                .iter()
                .any(|g| g.node == scroll_id && g.edge_fade.is_some()),
        "hover card trigger stays inside ancestor opacity and scroll edge fade: {card_trigger_groups:?}"
    );

    for (owner_id, item_id, expects_fade) in [
        (pop.stable_id(), pop_item.stable_id(), true),
        (menu.stable_id(), menu_item.stable_id(), true),
        (card.stable_id(), card_item.stable_id(), false),
    ] {
        // The owner node itself hosts the in-flow trigger, so its own filter_groups retain `root_id`,
        // while the floating surface primitive and hanging items isolate from outer ancestors.
        assert!(
            scene
                .filter_groups(owner_id)
                .iter()
                .any(|g| g.node == root_id),
            "in-flow trigger node {owner_id:?} keeps ancestor filter group"
        );
        let surface = scene
            .primitives()
            .find(|p| p.node == owner_id && p.id.slot == TRIGGERED_SURFACE_SLOT)
            .expect("triggered overlay surface");
        let groups = scene.primitive_opacity_groups(surface);
        assert!(
            groups
                .iter()
                .all(|g| g.node == owner_id && g.edge_fade.is_none() && g.filter.is_identity()),
            "isolated overlay {owner_id:?} strips outer opacity, filter & edge_fade while keeping only own fade: {groups:?}"
        );
        if expects_fade {
            assert!(
                groups
                    .iter()
                    .any(|g| g.node == owner_id && g.opacity > 0.0 && g.opacity < 1.0),
                "isolated overlay {owner_id:?} preserves its own open fade: {groups:?}"
            );
        } else {
            assert!(
                groups.is_empty(),
                "settled isolated overlay {owner_id:?} has no groups: {groups:?}"
            );
        }
        let item_prim = scene
            .primitives()
            .find(|p| p.node == item_id)
            .expect("menu item");
        assert_eq!(
            scene.primitive_opacity_groups(item_prim).as_ref(),
            groups.as_ref()
        );
        assert!(
            scene.filter_groups(item_id).is_empty(),
            "hanging menu item {item_id:?} drops ancestor filter groups"
        );
    }

    // Select open menu primitives isolate from ancestor opacity, filters, and edge fade while field stays inside.
    let mut saw_select_menu = false;
    let mut saw_select_field = false;
    for primitive in scene.primitives().filter(|p| p.node == select.stable_id()) {
        let slot_ns = primitive.id.slot >> 32;
        if primitive.id.slot == 8 || (9..=13).contains(&slot_ns) || (17..=18).contains(&slot_ns) {
            saw_select_menu = true;
            assert!(scene.primitive_opacity_groups(primitive).is_empty());
        } else if primitive.id.slot == 0 {
            saw_select_field = true;
            let groups = scene.primitive_opacity_groups(primitive);
            assert!(groups.iter().any(|g| g.node == root_id));
            assert!(
                groups
                    .iter()
                    .any(|g| g.node == scroll_id && g.edge_fade.is_some())
            );
        }
    }
    assert!(saw_select_field && saw_select_menu);

    // ContextMenu & Tooltip isolate their floating surface from ancestor opacity, filter, and edge fade.
    for floating_node in [ctx.stable_id(), tip.stable_id()] {
        assert!(
            scene.filter_groups(floating_node).is_empty(),
            "floating component {floating_node:?} drops ancestor filter"
        );
        assert!(scene.opacity_groups(floating_node).is_empty());
        let mut saw_primitive = false;
        for primitive in scene.primitives().filter(|p| p.node == floating_node) {
            saw_primitive = true;
            let prim_groups = scene.primitive_opacity_groups(primitive);
            assert!(
                prim_groups.is_empty(),
                "floating primitive {:?} of {floating_node:?} drops ancestor groups: {prim_groups:?}",
                primitive.id
            );
        }
        assert!(
            saw_primitive,
            "floating component {floating_node:?} painted primitives"
        );
    }
}

#[test]
fn opt_out_and_opt_in_control_ancestor_isolation_and_delta_updates() {
    let document = DocumentId::new(1).unwrap();
    let mut runtime = RuntimeDocument::new(document);
    let root = runtime
        .context_mut()
        .create_component(
            document,
            Stack::column(0.0).with_layout(|layout| {
                layout.opacity = Some(0.5);
                layout.paint.filter = Some(ColorFilter {
                    blur_radius: 3.0,
                    ..ColorFilter::default()
                });
                layout.width = Some(LengthSpec::Px(800.0));
                layout.height = Some(LengthSpec::Px(600.0));
            }),
        )
        .unwrap();

    let (_, (pop, pop_item, plain_fixed)) = runtime
        .context_mut()
        .mount_view(root.stable_id(), || {
            let pop = entity_ref::<Popover>();
            let pop_item = entity_ref::<nana_ui_runtime::Button>();
            let plain_fixed = entity_ref::<Stack>();
            let tree = widget(
                ScrollView::new(ScrollAxes::Vertical)
                    .edge_fade(10.0, 40.0)
                    .with_layout(|layout| {
                        layout.width = Some(LengthSpec::Px(300.0));
                        layout.height = Some(LengthSpec::Px(200.0));
                    }),
            )
            .children((
                widget(
                    Popover::new()
                        .trigger("Toggle")
                        .overlay_mount(OverlayMount::Inline)
                        .width(120.0),
                )
                .entity_ref(pop)
                .trigger(
                    widget(Stack::row(0.0).height(LengthSpec::Px(24.0))).children(text("Toggle")),
                )
                .children(button("Row").entity_ref(pop_item)),
                widget(Stack::column(0.0).with_layout(|layout| {
                    layout.position = PositionSpec::Fixed;
                    layout.width = Some(LengthSpec::Px(40.0));
                    layout.height = Some(LengthSpec::Px(40.0));
                }))
                .entity_ref(plain_fixed),
                widget(Stack::column(0.0).height(LengthSpec::Px(400.0))),
            ));
            with_refs(tree, (pop, pop_item, plain_fixed))
        })
        .unwrap();

    runtime
        .context_mut()
        .advance_animations(Duration::from_millis(1000));
    flush(&mut runtime);
    let cx = runtime.context_mut();
    cx.update_component(pop, |p, _| p.open = true).unwrap();
    cx.advance_animations(Duration::from_millis(2000));
    flush(&mut runtime);

    let root_id = root.stable_id();
    let scene = runtime.scene();
    // Plain Fixed stays in-flow visually and inherits outer structural opacity & filter.
    assert!(
        scene
            .opacity_groups(plain_fixed.stable_id())
            .iter()
            .any(|g| g.node == root_id && (g.opacity - 0.5).abs() < 1e-4)
    );
    assert!(
        scene
            .filter_groups(plain_fixed.stable_id())
            .iter()
            .any(|g| g.node == root_id)
    );

    // Popover with OverlayMount::Inline inherits outer opacity & filter.
    let item_prim = scene
        .primitives()
        .find(|p| p.node == pop_item.stable_id())
        .expect("inline popover item");
    assert!(
        scene
            .primitive_opacity_groups(item_prim)
            .iter()
            .any(|g| g.node == root_id && (g.opacity - 0.5).abs() < 1e-4)
    );
    assert!(
        scene
            .filter_groups(pop_item.stable_id())
            .iter()
            .any(|g| g.node == root_id)
    );

    // Incremental toggle: OverlayMount::Inline -> OverlayMount::NearestModal strips ancestor groups cleanly.
    runtime
        .context_mut()
        .update_component(pop, |p, _| p.overlay_mount = OverlayMount::NearestModal)
        .unwrap();
    flush(&mut runtime);
    let scene = runtime.scene();
    let item_prim = scene
        .primitives()
        .find(|p| p.node == pop_item.stable_id())
        .expect("nearest-modal popover item");
    assert!(scene.primitive_opacity_groups(item_prim).is_empty());
    assert!(scene.filter_groups(pop_item.stable_id()).is_empty());
}

#[test]
fn nearest_modal_overlay_escapes_inner_scroll_fade_stays_under_higher_modal_and_top_layer_escapes_all()
 {
    let document = DocumentId::new(1).unwrap();
    let mut runtime = RuntimeDocument::new(document);
    let root = runtime
        .context_mut()
        .create_component(document, Stack::column(0.0))
        .unwrap();

    let (
        _,
        (
            (host1, dialog1, scroll_in_modal, popover, pop_panel, footer_btn),
            (host2, dialog2, dialog2_body),
        ),
    ) = runtime
        .context_mut()
        .mount_view(root.stable_id(), || {
            let host1 = entity_ref::<OverlayHost>();
            let dialog1 = entity_ref::<Dialog>();
            let scroll_in_modal = entity_ref::<ScrollView>();
            let popover = entity_ref::<Popover>();
            let pop_panel = entity_ref::<nana_ui_runtime::Button>();
            let footer_btn = entity_ref::<nana_ui_runtime::Button>();
            let host2 = entity_ref::<OverlayHost>();
            let dialog2 = entity_ref::<Dialog>();
            let dialog2_body = entity_ref::<Stack>();

            let mut d1 = Dialog::new("Modal 1");
            {
                let layout = d1.style.layout_mut();
                layout.opacity = Some(0.5);
                layout.z_index = Some(2_000);
            }
            let d1_tree = widget(OverlayHost::new()).entity_ref(host1).children(
                widget(d1)
                    .entity_ref(dialog1)
                    .body(
                        widget(
                            ScrollView::new(ScrollAxes::Vertical)
                                .edge_fade(12.0, 48.0)
                                .scrollbars(ScrollbarVisibility::Hidden)
                                .with_layout(|layout| {
                                    layout.width = Some(LengthSpec::Px(320.0));
                                    layout.height = Some(LengthSpec::Px(140.0));
                                }),
                        )
                        .entity_ref(scroll_in_modal)
                        .children(
                            widget(Stack::column(0.0).height(LengthSpec::Px(400.0))).children((
                                widget(Stack::column(0.0).height(LengthSpec::Px(60.0))),
                                widget(
                                    Popover::new()
                                        .trigger("Inside modal")
                                        .width(160.0)
                                        .padding(6.0),
                                )
                                .entity_ref(popover)
                                .trigger(
                                    widget(Stack::row(0.0).height(LengthSpec::Px(24.0)))
                                        .children(text("Inside modal")),
                                )
                                .children({
                                    let mut pop_btn = nana_ui_runtime::Button::new("Action");
                                    let layout = pop_btn.style.layout_mut();
                                    layout.width = Some(LengthSpec::Px(140.0));
                                    layout.height = Some(LengthSpec::Px(80.0));
                                    widget(pop_btn).entity_ref(pop_panel)
                                }),
                            )),
                        ),
                    )
                    .footer(button("OK").entity_ref(footer_btn)),
            );
            let mut d2 = Dialog::new("Modal 2");
            d2.style.layout_mut().z_index = Some(3_000);
            let d2_tree = widget(OverlayHost::new()).entity_ref(host2).children(
                widget(d2).entity_ref(dialog2).body(
                    widget(Stack::column(0.0).with_layout(|layout| {
                        layout.width = Some(LengthSpec::Px(320.0));
                        layout.height = Some(LengthSpec::Px(160.0));
                        layout.background = Some([0.8, 0.2, 0.2, 1.0]);
                    }))
                    .entity_ref(dialog2_body),
                ),
            );
            with_refs(
                widget(Stack::column(0.0)).children((d1_tree, d2_tree)),
                (
                    (
                        host1,
                        dialog1,
                        scroll_in_modal,
                        popover,
                        pop_panel,
                        footer_btn,
                    ),
                    (host2, dialog2, dialog2_body),
                ),
            )
        })
        .unwrap();

    runtime
        .context_mut()
        .advance_animations(Duration::from_millis(1000));
    flush(&mut runtime);
    let cx = runtime.context_mut();
    assert!(cx.activate_overlay(host1, dialog1).unwrap());
    assert!(cx.activate_overlay(host2, dialog2).unwrap());
    flush(&mut runtime);
    runtime
        .context_mut()
        .update_component(popover, |p, _| p.open = true)
        .unwrap();
    runtime
        .context_mut()
        .advance_animations(Duration::from_millis(3000));
    flush(&mut runtime);

    let scene = runtime.scene();
    let d1_id = dialog1.stable_id();
    let scroll_id = scroll_in_modal.stable_id();
    let item_prim = scene
        .primitives()
        .find(|p| p.node == pop_panel.stable_id())
        .expect("popover panel primitive");
    let groups = scene.primitive_opacity_groups(item_prim);
    assert!(
        !groups.iter().any(|g| g.node == scroll_id),
        "NearestModal popover must escape inner scroll edge fade"
    );
    assert!(
        groups
            .iter()
            .any(|g| g.node == d1_id && (g.opacity - 0.5).abs() < 1e-4),
        "NearestModal popover must inherit enclosing ModalFrame's group: {groups:?}"
    );

    let pos = |target: StableNodeId| {
        scene
            .primitives()
            .position(|p| p.node == target)
            .expect("primitive in paint order")
    };
    let surface_pos = scene
        .primitives()
        .position(|p| p.node == popover.stable_id() && p.id.slot == TRIGGERED_SURFACE_SLOT)
        .expect("popover surface in paint order");
    assert!(
        surface_pos > pos(footer_btn.stable_id()) && pos(pop_panel.stable_id()) > surface_pos,
        "NearestModal popover must paint above its modal footer"
    );
    assert!(
        pos(pop_panel.stable_id()) < pos(dialog2_body.stable_id()),
        "NearestModal popover in Modal 1 must paint below higher Modal 2"
    );

    let panel_bounds = scene.draw_node_bounds(pop_panel.stable_id()).unwrap();
    let (hx, hy) = (
        panel_bounds.x + panel_bounds.width * 0.5,
        panel_bounds.y + panel_bounds.height * 0.5,
    );
    let nearest_hit = runtime.context().world().hit_test(document, hx, hy);
    assert_ne!(
        nearest_hit,
        Some(pop_panel.stable_id()),
        "higher Modal 2 must block clicks on Modal 1's NearestModal popover"
    );
    assert_ne!(
        nearest_hit,
        Some(popover.stable_id()),
        "higher Modal 2 must block clicks on Modal 1's NearestModal popover surface"
    );

    // Promote to OverlayMount::TopLayer: escapes Modal 1's opacity AND paints/hits above Modal 2.
    runtime
        .context_mut()
        .update_component(popover, |p, _| p.overlay_mount = OverlayMount::TopLayer)
        .unwrap();
    flush(&mut runtime);

    let scene = runtime.scene();
    let item_prim = scene
        .primitives()
        .find(|p| p.node == pop_panel.stable_id())
        .expect("TopLayer popover panel primitive");
    assert!(
        scene.primitive_opacity_groups(item_prim).is_empty(),
        "TopLayer popover must escape enclosing ModalFrame's opacity group"
    );
    let surface_pos = scene
        .primitives()
        .position(|p| p.node == popover.stable_id() && p.id.slot == TRIGGERED_SURFACE_SLOT)
        .expect("TopLayer popover surface in paint order");
    let d2_body_pos = scene
        .primitives()
        .position(|p| p.node == dialog2_body.stable_id())
        .expect("dialog2 body in paint order");
    let panel_pos = scene
        .primitives()
        .position(|p| p.node == pop_panel.stable_id())
        .expect("TopLayer popover panel in paint order");
    assert!(
        surface_pos > d2_body_pos && panel_pos > surface_pos,
        "TopLayer popover must paint above higher Modal 2"
    );
    assert_eq!(
        runtime.context().world().hit_test(document, hx, hy),
        Some(pop_panel.stable_id()),
        "TopLayer popover must win hit testing above higher Modal 2"
    );
}
