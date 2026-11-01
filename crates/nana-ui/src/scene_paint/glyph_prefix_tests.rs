//! While per-glyph presentation is all that moves, a frame puts back the
//! dest's text-free prefix from the frame before and draws only its text.
use std::sync::Arc;
use std::time::Duration;

use nana_ui_core::{GlyphEffect, LayoutStyle, LengthSpec, RichSpanStyle, RichText};
use nana_ui_runtime::{
    DocumentId, LayoutViewport, MutationQueue, NodeKind, NodeStyle, RichTextView, StableNodeId,
};
use nana_ui_scene::RuntimeDocument;

use super::tests::{readback_rgba, test_copy_target, test_device};
use super::*;

const SIZE: [u32; 2] = [240, 96];

fn block(width: f32, height: f32, background: [f32; 4]) -> NodeStyle {
    NodeStyle {
        layout: Arc::new(LayoutStyle {
            width: Some(LengthSpec::Px(width)),
            height: Some(LengthSpec::Px(height)),
            background: Some(background),
            ..LayoutStyle::default()
        }),
        ..NodeStyle::default()
    }
}

struct Waving {
    runtime: RuntimeDocument,
    shaper: crate::NanaTextShaper,
    panel: StableNodeId,
}

impl Waving {
    /// A panel holding a waving line of text. `quad_after` puts a box after
    /// the text in document order, which makes the frame an ordered one.
    fn new(quad_after: bool) -> Self {
        let document = DocumentId::new(1).unwrap();
        let mut runtime = RuntimeDocument::new(document);
        let mut queue = MutationQueue::new();
        let root = StableNodeId::new(1).unwrap();
        queue.create(root, document, NodeKind::Document);
        let panel = StableNodeId::new(2).unwrap();
        queue.create(panel, document, NodeKind::Element { tag: "div".into() });
        queue.insert(root, panel, None);
        queue.set_style(panel, block(240.0, 96.0, [0.1, 0.2, 0.6, 1.0]));
        runtime.context_mut().commit_mutations(queue).unwrap();
        let value = RichText::new("wavy text").with_span(0..4, RichSpanStyle::new().effect(0));
        let view = runtime
            .context_mut()
            .create_component(document, RichTextView::new(value).font_size(28.0))
            .unwrap();
        let mut queue = MutationQueue::new();
        queue.insert(panel, view.stable_id(), None);
        if quad_after {
            let after = StableNodeId::new(1_000).unwrap();
            queue.create(after, document, NodeKind::Element { tag: "div".into() });
            queue.insert(panel, after, None);
            queue.set_style(after, block(60.0, 20.0, [0.9, 0.6, 0.1, 1.0]));
        }
        runtime.context_mut().commit_mutations(queue).unwrap();
        runtime
            .context_mut()
            .set_rich_presentation(view, vec![GlyphEffect::wave(8.0)], None)
            .unwrap();
        let mut waving = Self {
            runtime,
            shaper: crate::NanaTextShaper::default(),
            panel,
        };
        for _ in 0..4 {
            waving.at(Duration::from_secs(100));
        }
        waving
    }

    /// The host's frame at `now`: the presentation clock, then a flush.
    fn at(&mut self, now: Duration) -> &UiScene {
        self.runtime.sync_presentation_clock(now);
        self.runtime
            .flush(
                LayoutViewport::new(SIZE[0] as f32, SIZE[1] as f32),
                &mut self.shaper,
            )
            .unwrap();
        self.runtime.scene()
    }

    fn restyle_panel(&mut self, background: [f32; 4]) {
        let mut queue = MutationQueue::new();
        queue.set_style(self.panel, block(240.0, 96.0, background));
        self.runtime.context_mut().commit_mutations(queue).unwrap();
    }
}

fn paint(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    painter: &mut SceneWgpuPainter,
    scene: &UiScene,
) -> (Vec<u8>, DestPassCounts) {
    let viewport = ScenePaintViewport {
        logical_size: [SIZE[0] as f32, SIZE[1] as f32],
        physical_size: SIZE,
        scale_factor: 1.0,
        scene_origin: [0.0, 0.0],
        target_origin: [0.0, 0.0],
        clear_color: [0.0, 0.0, 0.0, 1.0],
        clear: true,
    };
    let (texture, view) =
        test_copy_target(device, wgpu::TextureFormat::Rgba8Unorm, SIZE[0], SIZE[1]);
    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("nana-ui glyph prefix"),
    });
    painter
        .paint_encoder(scene, &mut encoder, &view, viewport, None, None)
        .unwrap();
    let counts = painter.last_dest_pass_counts.expect("an encoded frame");
    let pixels = readback_rgba(device, queue, encoder, &texture, SIZE[0], SIZE[1]);
    (pixels, counts)
}

fn prefix_allocated(painter: &SceneWgpuPainter) -> bool {
    painter
        .dest
        .as_ref()
        .is_some_and(DestTarget::prefix_allocated)
}

/// Frames at three instants of a wave: the first paints all, the second paints
/// all and keeps the prefix, the third puts it back. Each must match a painter
/// that paints that instant from nothing.
fn the_prefix_is_restored_with_the_same_pixels(quad_after: bool, interleaved: bool) {
    let (device, queue) = test_device();
    let format = wgpu::TextureFormat::Rgba8Unorm;
    let mut waving = Waving::new(quad_after);
    let mut painter = SceneWgpuPainter::for_test(format);
    let mut frames = Vec::new();
    for millis in [100_000u64, 100_130, 100_260, 100_390] {
        let scene = waving.at(Duration::from_millis(millis));
        assert!(scene.glyph_presentation_live(), "the wave keeps playing");
        let (pixels, counts) = paint(&device, &queue, &mut painter, scene);
        let (fresh, _) = paint(
            &device,
            &queue,
            &mut SceneWgpuPainter::for_test(format),
            scene,
        );
        assert!(
            pixels == fresh,
            "a frame at {millis} ms must paint what a fresh painter does ({counts:?})"
        );
        assert_eq!(counts.msaa_allocated, !interleaved);
        frames.push((pixels, counts));
    }
    assert_ne!(
        frames[2].0, frames[3].0,
        "the text moved between the frames"
    );
    let geometry = |counts: &DestPassCounts| {
        if interleaved {
            counts.color
        } else {
            counts.msaa
        }
    };
    let [first, second, third, fourth] = [0, 1, 2, 3].map(|index| frames[index].1);
    assert_eq!(
        (first.prefix_store, first.prefix_restore),
        (0, 0),
        "a new batch keeps nothing: {first:?}"
    );
    assert_eq!(
        (second.prefix_store, second.prefix_restore),
        (1, 0),
        "the batch painted a second time keeps its prefix: {second:?}"
    );
    for later in [third, fourth] {
        assert_eq!(
            (later.prefix_store, later.prefix_restore),
            (0, 1),
            "later frames put it back: {later:?}"
        );
        assert_eq!(later.msaa, 0, "and draw no geometry pass: {later:?}");
        assert!(
            geometry(&later) < geometry(&second),
            "nor the passes before the text: {later:?} against {second:?}"
        );
    }
}

#[test]
fn a_waving_ui_only_frame_draws_only_its_text_over_the_kept_geometry() {
    the_prefix_is_restored_with_the_same_pixels(false, false);
}

#[test]
fn a_waving_ordered_frame_draws_from_its_first_text_over_the_kept_prefix() {
    the_prefix_is_restored_with_the_same_pixels(true, true);
}

#[test]
fn a_change_under_the_text_paints_the_prefix_again() {
    let (device, queue) = test_device();
    let format = wgpu::TextureFormat::Rgba8Unorm;
    let mut waving = Waving::new(false);
    let mut painter = SceneWgpuPainter::for_test(format);
    for millis in [100_000u64, 100_100, 100_200] {
        let scene = waving.at(Duration::from_millis(millis));
        paint(&device, &queue, &mut painter, scene);
    }
    assert_eq!(painter.last_dest_pass_counts.unwrap().prefix_restore, 1);
    waving.restyle_panel([0.1, 0.6, 0.2, 1.0]);
    let scene = waving.at(Duration::from_millis(100_300));
    let (pixels, counts) = paint(&device, &queue, &mut painter, scene);
    assert_eq!(
        (counts.prefix_restore, counts.prefix_store),
        (0, 0),
        "a rebuilt batch paints everything: {counts:?}"
    );
    assert!(counts.msaa > 0, "{counts:?}");
    let (fresh, _) = paint(
        &device,
        &queue,
        &mut SceneWgpuPainter::for_test(format),
        scene,
    );
    assert!(pixels == fresh, "the new panel colour is painted");
    // The next frame of the new batch keeps it, the one after puts it back.
    let scene = waving.at(Duration::from_millis(100_400));
    assert_eq!(
        paint(&device, &queue, &mut painter, scene).1.prefix_store,
        1
    );
    let scene = waving.at(Duration::from_millis(100_500));
    let (pixels, counts) = paint(&device, &queue, &mut painter, scene);
    assert_eq!(counts.prefix_restore, 1);
    let (fresh, _) = paint(
        &device,
        &queue,
        &mut SceneWgpuPainter::for_test(format),
        scene,
    );
    assert!(pixels == fresh, "the restored prefix is the new panel's");
}

#[test]
fn the_kept_prefix_is_released_once_the_glyphs_stop() {
    let (device, queue) = test_device();
    let mut waving = Waving::new(false);
    let mut painter = SceneWgpuPainter::for_test(wgpu::TextureFormat::Rgba8Unorm);
    for millis in [100_000u64, 100_100] {
        let scene = waving.at(Duration::from_millis(millis));
        paint(&device, &queue, &mut painter, scene);
    }
    assert!(prefix_allocated(&painter));
    let view = waving
        .runtime
        .scene()
        .primitives()
        .find_map(|primitive| match &primitive.kind {
            nana_ui_scene::ScenePrimitiveKind::Text {
                presentation: Some(_),
                ..
            } => Some(primitive.node),
            _ => None,
        })
        .expect("the presented text");
    waving
        .runtime
        .context_mut()
        .clear_rich_presentation(view)
        .unwrap();
    for millis in [100_200u64, 100_300] {
        let scene = waving.at(Duration::from_millis(millis));
        assert!(!scene.glyph_presentation_live());
        paint(&device, &queue, &mut painter, scene);
    }
    assert!(
        !prefix_allocated(&painter),
        "a still frame holds no second copy of the dest"
    );
}
