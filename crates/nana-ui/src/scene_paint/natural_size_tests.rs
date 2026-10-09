//! Issue #263 from the host's side: a replaced box with no size of its own
//! lays out at the size its image decoded at, once the image is ready. The
//! painter notes the size while it paints; the host takes the notes after
//! presenting and commits them to the document it painted, as the built-in
//! host does after every present.
use super::*;
use nana_ui_core::{BackgroundImage, FlexDirection, LayoutStyle};
use nana_ui_runtime::{LayoutViewport, ReplacedMetadata, ReplacedResource};
use nana_ui_scene::RuntimeDocument;

const TARGET: RenderTargetId = RenderTargetId(263);
const SIDE: u32 = 120;

fn solid_png_data_url(width: u32, height: u32, rgba: [u8; 4]) -> String {
    use base64::Engine as _;
    let image = image::RgbaImage::from_pixel(width, height, image::Rgba(rgba));
    let mut bytes = std::io::Cursor::new(Vec::new());
    image
        .write_to(&mut bytes, image::ImageFormat::Png)
        .expect("encode png");
    format!(
        "data:image/png;base64,{}",
        base64::engine::general_purpose::STANDARD.encode(bytes.into_inner())
    )
}

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

/// A column holding an `<img>` with no width or height of its own, then a
/// fixed box the image pushes down once it has a size.
struct Page {
    document: RuntimeDocument,
    image: StableNodeId,
    below: StableNodeId,
}

impl Page {
    fn new(url: &str) -> Self {
        let id = DocumentId::new(263).unwrap();
        let node = |value| StableNodeId::new(value).unwrap();
        let (root, column, image, below) = (node(1), node(2), node(3), node(4));
        let mut queue = MutationQueue::new();
        queue.create(root, id, NodeKind::Document);
        let mut element = |node, parent, layout: LayoutStyle| {
            queue.create(node, id, NodeKind::Element { tag: "div".into() });
            queue.insert(parent, node, None);
            queue.set_style(
                node,
                NodeStyle {
                    layout: Arc::new(layout),
                    ..NodeStyle::default()
                },
            );
        };
        element(
            column,
            root,
            LayoutStyle {
                width: Some(LengthSpec::Px(SIDE as f32)),
                direction: Some(FlexDirection::Column),
                ..LayoutStyle::default()
            },
        );
        let mut replaced = LayoutStyle::default();
        replaced.paint.content_image = Some(BackgroundImage::url(url));
        element(image, column, replaced);
        element(
            below,
            column,
            LayoutStyle {
                width: Some(LengthSpec::Px(10.0)),
                height: Some(LengthSpec::Px(10.0)),
                ..LayoutStyle::default()
            },
        );
        let mut document = RuntimeDocument::new(id);
        document.context_mut().commit_mutations(queue).unwrap();
        let mut page = Self {
            document,
            image,
            below,
        };
        page.frame();
        page
    }

    fn frame(&mut self) {
        self.document
            .flush(
                LayoutViewport::new(SIDE as f32, SIDE as f32),
                &mut crate::NanaTextShaper::default(),
            )
            .unwrap();
    }

    fn layout(&self, id: StableNodeId) -> (f32, f32, f32, f32) {
        let layout = self.document.context().world().layout_box(id).unwrap();
        (layout.x, layout.y, layout.width, layout.height)
    }

    /// Paint into a fresh target of the shared test device; its pixels.
    fn paint(&self, painter: &mut SceneWgpuPainter) -> Vec<u8> {
        let (device, queue) = test_device();
        let format = wgpu::TextureFormat::Rgba8Unorm;
        let (texture, view) = test_copy_target(&device, format, SIDE, SIDE);
        let mut encoder = device.create_command_encoder(&Default::default());
        painter
            .paint_target_encoder(
                TARGET,
                self.document.scene(),
                &mut encoder,
                &view,
                viewport(),
                None,
                None,
            )
            .unwrap();
        readback_rgba(&device, &queue, encoder, &texture, SIDE, SIDE)
    }

    /// What the host does after presenting.
    fn commit(&mut self, sizes: &[ImageNaturalSize]) -> bool {
        commit_image_natural_sizes(self.document.context_mut(), sizes).unwrap()
    }
}

#[test]
fn an_image_with_no_size_lays_out_at_the_size_it_decoded_at() {
    let url = solid_png_data_url(40, 30, [255, 0, 0, 255]);
    let mut page = Page::new(&url);
    let (image, below) = (page.image, page.below);
    let (_, _, width, height) = page.layout(image);
    assert_eq!(width * height, 0.0, "no size yet, so no area");
    assert_eq!(page.layout(below).1, 0.0);

    // The box has no area, so nothing draws it; the painter still starts its
    // image, and notes the size the image decoded at.
    let mut painter = SceneWgpuPainter::for_test(wgpu::TextureFormat::Rgba8Unorm);
    page.paint(&mut painter);
    let sizes = painter.take_image_natural_sizes(Some(TARGET));
    assert_eq!(
        sizes,
        [ImageNaturalSize {
            url: Arc::from(url.as_str()),
            width: 40,
            height: 30,
        }]
    );
    assert!(
        painter.take_image_natural_sizes(None).is_empty(),
        "the notes are the target's"
    );

    // Committed after presenting; the next frame lays the box out at it.
    assert!(page.commit(&sizes));
    let resource = ReplacedResource::Url(Arc::from(url.as_str()));
    assert_eq!(
        page.document.context().world().replaced_metadata(&resource),
        Some(ReplacedMetadata::new(40.0, 30.0))
    );
    page.frame();
    assert_eq!(page.layout(image), (0.0, 0.0, 40.0, 30.0));
    assert_eq!(
        page.layout(below).1,
        30.0,
        "what follows the image moves down"
    );

    // The image paints at that size, and nothing new is handed over.
    let pixels = page.paint(&mut painter);
    let inside = pixel(&pixels, SIDE, 20, 15);
    assert!(inside[0] > 200 && inside[1] < 50, "{inside:?}");
    let beside = pixel(&pixels, SIDE, 50, 15);
    assert!(beside[0] < 50, "40 wide, not stretched: {beside:?}");
    assert!(painter.take_image_natural_sizes(Some(TARGET)).is_empty());
    page.paint(&mut painter);
    assert!(painter.take_image_natural_sizes(Some(TARGET)).is_empty());

    // A size the document has, or of an image no node of it shows, is not
    // committed again.
    assert!(!page.commit(&sizes));
    let elsewhere = ImageNaturalSize {
        url: Arc::from("asset://elsewhere.png"),
        width: 8,
        height: 8,
    };
    assert!(!page.commit(std::slice::from_ref(&elsewhere)));
    assert_eq!(
        page.document
            .context()
            .world()
            .replaced_metadata(&ReplacedResource::Url(elsewhere.url)),
        None
    );
}

#[test]
fn a_remote_image_lays_its_box_out_once_it_arrives() {
    let (_, path) = blue_tile_fixture_png();
    let server = LocalPngServer::serve(std::fs::read(path).unwrap());
    let mut page = Page::new(&server.url);
    let image = page.image;
    let mut painter = SceneWgpuPainter::for_test(wgpu::TextureFormat::Rgba8Unorm);
    painter.set_resource_fetch_host(Some(crate::scene_paint::image_url::loopback_fetch_host()));
    let (wake, awoken) = std::sync::mpsc::channel();
    painter.set_image_waker(Arc::new(move || {
        let _ = wake.send(());
    }));

    page.paint(&mut painter);
    assert!(
        painter.has_pending_images(),
        "the box with no area asked for its image"
    );
    assert!(painter.take_image_natural_sizes(Some(TARGET)).is_empty());
    let deadline = Instant::now() + std::time::Duration::from_secs(10);
    let mut sizes = Vec::new();
    while sizes.is_empty() {
        awoken
            .recv_timeout(deadline.saturating_duration_since(Instant::now()))
            .expect("the image arrives");
        // The paint the waker asks for collects the result.
        page.paint(&mut painter);
        sizes = painter.take_image_natural_sizes(Some(TARGET));
    }
    assert_eq!(
        sizes,
        [ImageNaturalSize {
            url: Arc::from(server.url.as_str()),
            width: 8,
            height: 8,
        }]
    );
    let (_, _, width, height) = page.layout(image);
    assert_eq!(width * height, 0.0, "not before the host commits it");
    assert!(page.commit(&sizes));
    page.frame();
    assert_eq!(page.layout(image), (0.0, 0.0, 8.0, 8.0));
    let pixels = page.paint(&mut painter);
    assert!(pixel(&pixels, SIDE, 4, 4)[2] > 200, "the blue tile paints");
    assert!(painter.take_image_natural_sizes(Some(TARGET)).is_empty());
}

/// A box with no area starts its image only where it would show: inside the
/// clip it sits in, not in a part of the page an ancestor clips away, which
/// loads once it scrolls into view, as a box with area does.
#[test]
fn a_box_with_no_area_starts_its_image_only_inside_its_clip() {
    let shown = solid_png_data_url(6, 4, [0, 255, 0, 255]);
    let clipped = solid_png_data_url(5, 3, [0, 0, 255, 255]);
    let replaced = |value, y, url: &str| {
        let surface = nana_ui_scene::QuadSurfacePaint {
            content_image: Some(BackgroundImage::url(url)),
            ..Default::default()
        };
        let mut node = paint_surface_quad_node(value, 8.0, y, 0.0, 0.0, [0.0; 4], surface);
        node.parent = Some(StableNodeId::new(1).unwrap());
        node
    };
    let clip = extracted_div(
        1,
        &[2, 3],
        0.0,
        0.0,
        32.0,
        32.0,
        LayoutStyle {
            overflow_x: OverflowSpec::Hidden,
            overflow_y: OverflowSpec::Hidden,
            ..LayoutStyle::default()
        },
        None,
    );
    let mut scene = UiScene::new();
    scene.apply_delta(
        [clip, replaced(2, 8.0, &shown), replaced(3, 48.0, &clipped)],
        [],
    );
    let (device, queue) = test_device();
    let format = wgpu::TextureFormat::Rgba8Unorm;
    let (texture, view) = test_copy_target(&device, format, SIDE, SIDE);
    let mut painter = SceneWgpuPainter::for_test(format);
    let mut encoder = device.create_command_encoder(&Default::default());
    painter
        .paint_encoder(&scene, &mut encoder, &view, viewport(), None, None)
        .unwrap();
    readback_rgba(&device, &queue, encoder, &texture, SIDE, SIDE);
    assert!(painter.url_cache.contains_retained(&shown));
    assert!(!painter.url_cache.contains_retained(&clipped));
    assert_eq!(
        painter.take_image_natural_sizes(None),
        [ImageNaturalSize {
            url: Arc::from(shown.as_str()),
            width: 6,
            height: 4,
        }]
    );
}
