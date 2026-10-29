//! Issue #202 from the host's side: a host texture that moves to another
//! formatting context keeps its node, its descriptor and its bound texture.
//! The painter follows it to its new box without binding the texture again.
//! And an image's `object-fit` and a host texture's fit are one contract:
//! the painter's `url()` path sizes a picture as the host-texture path does.
use super::*;
use nana_ui_core::{DisplaySpec, FlexDirection, GridTrack, LayoutStyle};
use nana_ui_runtime::{HOST_TEXTURE_RENDERER, LayoutViewport, ParticipationKind};
use nana_ui_scene::RuntimeDocument;

const TARGET: RenderTargetId = RenderTargetId(202);
const SIDE: u32 = 120;
const CELL: f32 = 40.0;

fn viewport() -> ScenePaintViewport {
    ScenePaintViewport {
        logical_size: [SIDE as f32; 2],
        physical_size: [SIDE; 2],
        scale_factor: 1.0,
        scene_origin: [0.0; 2],
        target_origin: [0.0; 2],
        clear_color: [0.0, 0.0, 0.0, 1.0],
        clear: true,
    }
}

fn styled(layout: LayoutStyle) -> NodeStyle {
    NodeStyle {
        layout: Arc::new(layout),
        ..NodeStyle::default()
    }
}

fn container(display: DisplaySpec, direction: Option<FlexDirection>) -> LayoutStyle {
    LayoutStyle {
        display: Some(display),
        direction,
        width: Some(LengthSpec::Px(SIDE as f32)),
        grid_columns: (display == DisplaySpec::Grid)
            .then(|| vec![GridTrack::Px(CELL), GridTrack::Px(CELL)]),
        ..LayoutStyle::default()
    }
}

fn cell() -> LayoutStyle {
    LayoutStyle {
        width: Some(LengthSpec::Px(CELL)),
        height: Some(LengthSpec::Px(CELL)),
        ..LayoutStyle::default()
    }
}

#[test]
fn a_host_texture_moved_across_contexts_keeps_its_binding() {
    let (device, queue) = test_device();
    let format = wgpu::TextureFormat::Rgba8Unorm;
    let source = solid_texture_view(&device, &queue, format, 8, 8, wgpu::Color::GREEN);
    let registry = register_host_texture("live", &source, 8, 8);

    let id = DocumentId::new(202).unwrap();
    let node = |value| StableNodeId::new(value).unwrap();
    let (root, parent, filler, texture) = (node(1), node(2), node(3), node(4));
    let mut mutations = MutationQueue::new();
    mutations.create(root, id, NodeKind::Document);
    for (child, owner) in [(parent, root), (filler, parent), (texture, parent)] {
        mutations.create(child, id, NodeKind::Element { tag: "div".into() });
        mutations.insert(owner, child, None);
    }
    mutations.set_style(
        parent,
        styled(container(DisplaySpec::Flex, Some(FlexDirection::Column))),
    );
    mutations.set_style(filler, styled(cell()));
    mutations.set_style(texture, styled(cell()));
    mutations.set_custom_render(
        texture,
        Some(CustomRenderNode::new(HOST_TEXTURE_RENDERER, "live", 1)),
    );
    let mut document = RuntimeDocument::new(id);
    document.context_mut().commit_mutations(mutations).unwrap();

    let mut painter = SceneWgpuPainter::for_test(format);
    let (target, view) = test_copy_target(&device, format, SIDE, SIDE);
    let mut paint = |document: &mut RuntimeDocument| {
        document
            .flush(
                LayoutViewport::new(SIDE as f32, SIDE as f32),
                &mut crate::NanaTextShaper::default(),
            )
            .unwrap();
        let mut encoder = device.create_command_encoder(&Default::default());
        painter
            .paint_target_encoder(
                TARGET,
                document.scene(),
                &mut encoder,
                &view,
                viewport(),
                Some(&registry),
                None,
            )
            .unwrap();
        let pixels = readback_rgba(&device, &queue, encoder, &target, SIDE, SIDE);
        (pixels, painter.last_gpu_work().unwrap())
    };

    let (pixels, _) = paint(&mut document);
    let descriptor = document
        .context()
        .world()
        .custom_render(texture)
        .cloned()
        .unwrap();
    // Below the filler in the column.
    assert!(is_green_slot(pixel(&pixels, SIDE, 20, 60)));
    assert!(!is_green_slot(pixel(&pixels, SIDE, 60, 20)));

    for (layout, part, at, empty) in [
        // Beside the filler, in the grid's second column.
        (
            container(DisplaySpec::Grid, None),
            ParticipationKind::GridItem,
            (60, 20),
            (20, 60),
        ),
        // Back below it, then beside it again in a row.
        (
            container(DisplaySpec::Flex, Some(FlexDirection::Column)),
            ParticipationKind::FlexItem,
            (20, 60),
            (60, 20),
        ),
        (
            container(DisplaySpec::Flex, Some(FlexDirection::Row)),
            ParticipationKind::FlexItem,
            (60, 20),
            (20, 60),
        ),
    ] {
        let mut mutations = MutationQueue::new();
        mutations.set_style(parent, styled(layout));
        document.context_mut().commit_mutations(mutations).unwrap();
        let (pixels, work) = paint(&mut document);
        let world = document.context().world();
        assert_eq!(
            world.layout_node(texture).unwrap().participation,
            Some(part)
        );
        let now = world.custom_render(texture).unwrap();
        assert_eq!(now, &descriptor);
        assert!(Arc::ptr_eq(&now.resource, &descriptor.resource));
        assert!(is_green_slot(pixel(&pixels, SIDE, at.0, at.1)), "{part:?}");
        assert!(
            !is_green_slot(pixel(&pixels, SIDE, empty.0, empty.1)),
            "{part:?}"
        );
        // Moving the box rebinds nothing: no new uniform or bind group.
        assert_eq!(work.gpu_buffer_reallocations, 0, "{part:?} {work:?}");
    }
}

#[test]
fn an_image_and_a_host_texture_share_one_fit() {
    use nana_ui_core::{BackgroundPosition, BackgroundRepeat, ContentFit};
    for fit in [
        ContentFit::Fill,
        ContentFit::Contain,
        ContentFit::Cover,
        ContentFit::None,
        ContentFit::ScaleDown,
    ] {
        for ((box_w, box_h), (tex_w, tex_h)) in [
            ((200.0, 100.0), (50.0, 50.0)),
            ((100.0, 200.0), (400.0, 100.0)),
            ((120.0, 80.0), (30.0, 20.0)),
            ((90.0, 60.0), (300.0, 300.0)),
            ((64.0, 64.0), (64.0, 64.0)),
            ((150.0, 40.0), (20.0, 90.0)),
        ] {
            // Where the host-texture path draws (`ScenePrimitiveKind::Custom`).
            let host =
                nana_ui_core::LogicalRect::new(0.0, 0.0, box_w, box_h).fitted(tex_w, tex_h, fit);
            // Where the `url()` path draws an `<img>` with this `object-fit`.
            let [x, y, width, height] = super::quad::url_dest_rect(
                fit.into(),
                None,
                None,
                BackgroundPosition::center(),
                BackgroundRepeat::NoRepeat,
                box_w,
                box_h,
                tex_w,
                tex_h,
            );
            let image = [x * box_w, y * box_h, width * box_w, height * box_h];
            let host = [host.x, host.y, host.width, host.height];
            for (image, host) in image.iter().zip(host) {
                assert!(
                    (image - host).abs() < 1e-3,
                    "{fit:?} box {box_w}x{box_h} source {tex_w}x{tex_h}: image {image} host {host}"
                );
            }
        }
    }
}
