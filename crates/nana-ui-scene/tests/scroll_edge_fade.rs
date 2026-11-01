//! A scroll view's edge fade as the scene hands it to a painter.
//!
//! The view isolates its subtree into one destination group that carries
//! the fade bands for the current offset. Its bars composite outside the
//! fade, and a view without one stays out of any group.

use nana_ui_core::{LengthSpec, ScrollbarVisibility};
use nana_ui_runtime::{
    ComputedStyle, DocumentId, Entity, LayoutViewport, ScrollAxes, ScrollOffset, ScrollView,
    StableNodeId, Stack, TextContent, TextMetrics, TextShapeConstraints, TextShaper,
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
