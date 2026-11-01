//! A scroll view's edge fade over a transparent target: the content's alpha
//! ramps to zero at the scrollport's edges, nothing is painted where it
//! fades, and the bars stay whole.
use super::tests::{readback_rgba, test_copy_target, test_device};
use super::*;
use nana_ui_core::{LengthSpec, ScrollbarVisibility};
use nana_ui_runtime::{
    ComputedStyle, DocumentId, Entity, LayoutViewport, ScrollAxes, ScrollOffset, ScrollView,
    StableNodeId, Stack, TextContent, TextMetrics, TextShapeConstraints, TextShaper,
};
use nana_ui_scene::RuntimeDocument;

const SIZE: [u32; 2] = [320, 240];

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
        .flush(
            LayoutViewport::new(SIZE[0] as f32, SIZE[1] as f32),
            &mut NoText,
        )
        .unwrap();
}

/// A 300×200 scroll view at the origin holding `rows` rows 100 tall, each
/// filled with `row` when given.
fn scroll_view(
    fade: Option<(f32, f32)>,
    bars: ScrollbarVisibility,
    rows: usize,
    row: Option<[f32; 4]>,
) -> (RuntimeDocument, Entity<ScrollView>) {
    let document = DocumentId::new(1).unwrap();
    let mut runtime = RuntimeDocument::new(document);
    let cx = runtime.context_mut();
    let mut view = ScrollView::new(ScrollAxes::Vertical)
        .scrollbars(bars)
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
    for _ in 0..rows {
        let entity = cx
            .create_component(
                document,
                Stack::column(0.0).with_layout(|layout| {
                    layout.height = Some(LengthSpec::Px(100.0));
                    layout.background = row;
                }),
            )
            .unwrap();
        cx.append_child(column, entity).unwrap();
    }
    flush(&mut runtime);
    (runtime, scroll)
}

fn scroll_to(runtime: &mut RuntimeDocument, scroll: Entity<ScrollView>, y: f32) {
    runtime
        .context_mut()
        .scroll_to(scroll, ScrollOffset { x: 0.0, y })
        .unwrap();
    flush(runtime);
}

/// The frame over a fully transparent target.
fn paint(device: &wgpu::Device, queue: &wgpu::Queue, scene: &UiScene) -> Vec<u8> {
    let mut painter = SceneWgpuPainter::for_test(wgpu::TextureFormat::Rgba8Unorm);
    let viewport = ScenePaintViewport {
        logical_size: [SIZE[0] as f32, SIZE[1] as f32],
        physical_size: SIZE,
        scale_factor: 1.0,
        scene_origin: [0.0, 0.0],
        target_origin: [0.0, 0.0],
        clear_color: [0.0, 0.0, 0.0, 0.0],
        clear: true,
    };
    let (texture, view) =
        test_copy_target(device, wgpu::TextureFormat::Rgba8Unorm, SIZE[0], SIZE[1]);
    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("nana-ui scroll edge fade probe"),
    });
    painter
        .paint_encoder(scene, &mut encoder, &view, viewport, None, None)
        .unwrap();
    readback_rgba(device, queue, encoder, &texture, SIZE[0], SIZE[1])
}

fn pixel(frame: &[u8], x: u32, y: u32) -> [u8; 4] {
    let at = ((y * SIZE[0] + x) * 4) as usize;
    frame[at..at + 4].try_into().unwrap()
}

/// Alpha down column `x` against the band ramp: `cover(y)` at each row's
/// centre, within rounding.
fn assert_alpha(frame: &[u8], x: u32, rows: std::ops::Range<u32>, cover: impl Fn(f32) -> f32) {
    for y in rows {
        let expected = cover(y as f32 + 0.5).clamp(0.0, 1.0) * 255.0;
        let actual = f32::from(pixel(frame, x, y)[3]);
        assert!(
            (actual - expected).abs() <= 3.0,
            "row {y}: alpha {actual}, expected {expected:.1}"
        );
    }
}

#[test]
fn the_content_fades_to_transparent_at_the_scrollport_edges() {
    let (device, queue) = test_device();
    let white = Some([1.0, 1.0, 1.0, 1.0]);
    let (mut runtime, scroll) =
        scroll_view(Some((14.0, 64.0)), ScrollbarVisibility::Hidden, 10, white);
    let rest = paint(&device, &queue, runtime.scene());
    // At rest only the bottom band fades; the top edge is solid.
    assert_alpha(&rest, 100, 0..136, |_| 1.0);
    assert_alpha(&rest, 100, 136..200, |y| (200.0 - y) / 64.0);
    // Nothing reaches past the scrollport, and no band is painted there.
    assert_alpha(&rest, 100, 200..SIZE[1], |_| 0.0);
    assert_alpha(&rest, 310, 0..SIZE[1], |_| 0.0);

    scroll_to(&mut runtime, scroll, 7.0);
    let started = paint(&device, &queue, runtime.scene());
    // Seven pixels scrolled away: the top band is seven deep, not fourteen.
    assert_alpha(&started, 100, 0..7, |y| y / 7.0);
    assert_alpha(&started, 100, 7..136, |_| 1.0);

    scroll_to(&mut runtime, scroll, 400.0);
    let middle = paint(&device, &queue, runtime.scene());
    assert_alpha(&middle, 100, 0..14, |y| y / 14.0);
    assert_alpha(&middle, 100, 14..136, |_| 1.0);
    assert_alpha(&middle, 100, 136..200, |y| (200.0 - y) / 64.0);

    scroll_to(&mut runtime, scroll, 800.0);
    let end = paint(&device, &queue, runtime.scene());
    assert_alpha(&end, 100, 14..200, |_| 1.0);

    // The same content without a fade paints solid to both edges.
    let (mut plain, scroll) = scroll_view(None, ScrollbarVisibility::Hidden, 10, white);
    scroll_to(&mut plain, scroll, 400.0);
    let plain = paint(&device, &queue, plain.scene());
    assert_alpha(&plain, 100, 0..200, |_| 1.0);
}

#[test]
fn the_bars_are_not_faded() {
    let (device, queue) = test_device();
    // 300 of content in a 200 port, scrolled halfway: the thumb reaches
    // down into the bottom band.
    let paint_bars = |fade| {
        let (mut runtime, scroll) = scroll_view(fade, ScrollbarVisibility::Always, 3, None);
        scroll_to(&mut runtime, scroll, 50.0);
        let thumb = match runtime
            .context()
            .world()
            .component_geometry(scroll.stable_id())
        {
            Some(nana_ui_runtime::ComponentGeometry::Scrollbar {
                vertical: Some(bar),
                ..
            }) => bar.thumb,
            other => panic!("a vertical bar: {other:?}"),
        };
        (paint(&device, &queue, runtime.scene()), thumb)
    };
    let (faded, thumb) = paint_bars(Some((14.0, 64.0)));
    let (plain, _) = paint_bars(None);
    let x = (thumb.x + thumb.width / 2.0) as u32;
    let y = (thumb.y + thumb.height - 3.0) as u32;
    assert!(y > 150, "the probe sits in the bottom band: {thumb:?}");
    let bar = pixel(&plain, x, y);
    assert!(bar[3] > 0, "the thumb is drawn at {x},{y}");
    assert_eq!(pixel(&faded, x, y), bar);
}
