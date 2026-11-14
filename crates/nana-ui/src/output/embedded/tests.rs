use super::*;
use crate::{ScenePaintViewport, SceneWgpuPainter, test_gpu};
use nana_gpu::{__framework, GpuTextureDescriptor, GpuTextureFormat, GpuTextureUsages};
use nana_ui_core::{LayoutStyle, OverflowSpec, PaintTransform};
use nana_ui_runtime::{
    ComputedStyle, CustomRenderNode, ExtractedNode, LayoutBox, NodeKind, NodeStyle,
};
use nana_ui_scene::UiScene;
use std::time::{Duration, Instant};

const EXTENT: [u32; 2] = [64, 64];

fn viewport() -> ScenePaintViewport {
    ScenePaintViewport {
        logical_size: [64.0, 64.0],
        physical_size: EXTENT,
        scale_factor: 1.0,
        scene_origin: [0.0; 2],
        target_origin: [0.0; 2],
        clear_color: [0.0, 0.0, 0.0, 1.0],
        clear: true,
    }
}

fn node(id: u64, bounds: [f32; 4], background: Option<[f32; 4]>) -> ExtractedNode {
    ExtractedNode {
        layout_result: None,
        custom_paint: None,
        chrome_radii: Default::default(),
        id: StableNodeId::new(id).unwrap(),
        kind: Arc::new(NodeKind::Element { tag: "div".into() }),
        parent: None,
        visual_parent: None,
        overlay_visual_parent: None,
        children: Arc::new(Vec::new()),
        layout: LayoutBox {
            x: bounds[0],
            y: bounds[1],
            width: bounds[2],
            height: bounds[3],
        },
        local_layout: Default::default(),
        layout_frame: nana_ui_runtime::LayoutFrame::Page,
        scroll_offset: Default::default(),
        source_style: NodeStyle::default(),
        style: Arc::new(ComputedStyle {
            background,
            ..Default::default()
        }),
        text: None,
        text_metrics: None,
        text_layout: None,
        text_preserve_lines: false,
        z_index: 0,
        focused: false,
        editable: false,
        text_spans: Vec::new(),
        standard_visual: None,
        component_geometry: None,
        standard_visual_foreground: None,
        custom_render: None,
        drop_hover: None,
        document_text_selection: Vec::new(),
        document_text_selection_color: [0.0; 4],
        compositor: Default::default(),
        rich_text: None,
        rich_editor: None,
        glyph_presentation: None,
    }
}

fn child_scene(color: [f32; 4]) -> UiScene {
    let mut scene = UiScene::new();
    scene.apply_delta([node(1, [0.0, 0.0, 64.0, 64.0], Some(color))], []);
    scene
}

fn surface() -> super::super::ExternalSurface {
    super::super::ExternalSurface::new(
        &test_gpu::context(),
        super::super::ExternalSurfaceConfig::new(EXTENT, GpuTextureFormat::RGBA8_UNORM_SRGB),
    )
    .unwrap()
}

fn render(
    surface: &mut super::super::ExternalSurface,
    scene: &UiScene,
    revision: u64,
) -> ExternalFrame {
    let deadline = Instant::now() + Duration::from_secs(30);
    let mut wanted = None;
    loop {
        surface.poll();
        if wanted.is_none() {
            match surface
                .render(scene, revision, viewport(), None, None)
                .unwrap()
            {
                super::super::ExternalRenderOutcome::Submitted {
                    content_revision, ..
                }
                | super::super::ExternalRenderOutcome::Reused {
                    content_revision, ..
                } => wanted = Some(content_revision),
                super::super::ExternalRenderOutcome::Deferred => {}
            }
        }
        if let Ok(frame) = surface.sample()
            && Some(frame.content_revision()) == wanted
        {
            return frame;
        }
        assert!(Instant::now() < deadline, "child output never completed");
        std::thread::sleep(Duration::from_millis(1));
    }
}

fn parent_anchor(offset: f32, opacity: f32) -> ExtractedNode {
    let mut parent = node(1, [8.0, 8.0, 40.0, 40.0], None);
    parent.children = Arc::new(vec![StableNodeId::new(2).unwrap()]);
    parent.source_style.layout = Arc::new(LayoutStyle {
        overflow_x: OverflowSpec::Hidden,
        overflow_y: OverflowSpec::Hidden,
        transform: Some(PaintTransform {
            e: offset,
            ..Default::default()
        }),
        opacity: Some(opacity),
        ..Default::default()
    });
    parent
}

fn parent_scene(offset: f32, opacity: f32) -> UiScene {
    let parent = parent_anchor(offset, opacity);
    let mut embedded = node(2, [0.0, 0.0, 64.0, 64.0], None);
    embedded.parent = Some(parent.id);
    embedded.custom_render = Some(CustomRenderNode::new("nana.host-texture", "child", 1));
    let overlay = node(3, [24.0, 24.0, 12.0, 12.0], Some([0.0, 0.0, 1.0, 1.0]));
    let mut scene = UiScene::new();
    scene.apply_delta([parent, embedded, overlay], []);
    scene
}

fn target(gpu: &GpuContext) -> GpuTexture {
    gpu.create_texture(&GpuTextureDescriptor {
        label: Some("embedded parent test"),
        width: 64,
        height: 64,
        format: GpuTextureFormat::RGBA8_UNORM,
        usage: GpuTextureUsages::RENDER_TARGET | GpuTextureUsages::COPY_SRC,
    })
    .unwrap()
}

fn read_pixels(gpu: &GpuContext, texture: &GpuTexture) -> Vec<u8> {
    // Explicit test-only readback, kept outside all output work observations.
    let device = __framework::device(gpu);
    let buffer = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("embedded test readback"),
        size: 256 * 64,
        usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    let mut frame = gpu.begin_frame("embedded test readback");
    __framework::encoder(&mut frame).copy_texture_to_buffer(
        __framework::texture(texture).as_image_copy(),
        wgpu::TexelCopyBufferInfo {
            buffer: &buffer,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(256),
                rows_per_image: Some(64),
            },
        },
        wgpu::Extent3d {
            width: 64,
            height: 64,
            depth_or_array_layers: 1,
        },
    );
    frame.submit();
    let (send, receive) = std::sync::mpsc::channel();
    buffer
        .slice(..)
        .map_async(wgpu::MapMode::Read, move |result| {
            send.send(result).unwrap();
        });
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        gpu.poll();
        if let Ok(result) = receive.try_recv() {
            result.unwrap();
            break;
        }
        assert!(Instant::now() < deadline, "test readback timed out");
        std::thread::sleep(Duration::from_millis(1));
    }
    buffer.slice(..).get_mapped_range().unwrap().to_vec()
}

fn pixel(pixels: &[u8], x: usize, y: usize) -> &[u8] {
    &pixels[(y * 64 + x) * 4..(y * 64 + x) * 4 + 4]
}

#[test]
fn extents_and_sampling_metadata_are_explicit() {
    assert!(validate_extents([320.0, 180.0], [640, 360]).is_ok());
    assert_eq!(
        validate_extents([0.0, 1.0], [1, 1]),
        Err(EmbeddedMetadataError::InvalidLogicalExtent)
    );
    assert_eq!(
        validate_extents([f32::NAN, 1.0], [1, 1]),
        Err(EmbeddedMetadataError::InvalidLogicalExtent)
    );
    assert_eq!(
        validate_extents([1.0; 2], [0, 1]),
        Err(EmbeddedMetadataError::InvalidTextureExtent)
    );
}

#[test]
fn static_child_in_120_and_240_submitted_parent_frames_has_zero_producer_work() {
    let gpu = test_gpu::context();
    let mut child = surface();
    let scene = child_scene([1.0, 0.0, 0.0, 1.0]);
    let mut embedded =
        EmbeddedSurfaceNode::from_frame(render(&mut child, &scene, 0), [64.0; 2]).unwrap();
    let registry = HostTextureRegistry::new();
    embedded.register_host_texture(&registry, "child", HostTextureAlphaMode::Premultiplied);
    let identity = embedded.host_texture().instance_identity();
    let registry_revision = registry.revision();
    let target = target(&gpu);
    let mut painter = SceneWgpuPainter::new(&gpu, GpuTextureFormat::RGBA8_UNORM);
    let mut parent = parent_scene(0.0, 0.5);
    for count in [120, 240] {
        for index in 0..count {
            parent.apply_delta(
                [parent_anchor(
                    (index % 2) as f32 * 2.0,
                    if index % 2 == 0 { 0.5 } else { 0.75 },
                )],
                [],
            );
            assert!(matches!(
                child.render(&scene, 0, viewport(), None, None).unwrap(),
                super::super::ExternalRenderOutcome::Reused { .. }
            ));
            let work = child.last_work();
            assert_eq!(
                (
                    work.content_revisions,
                    work.target_recreates,
                    work.extra_passes,
                    work.gpu_copies,
                    work.gpu_copy_bytes
                ),
                (0, 0, 0, 0, 0)
            );
            let mut frame = gpu.begin_frame("active parent static child");
            painter
                .paint(
                    &parent,
                    &mut frame,
                    &target.render_target().unwrap(),
                    viewport(),
                    Some(&registry),
                    None,
                )
                .unwrap();
            let submitted = frame.submit();
            embedded.bind_to_submission(&gpu, &submitted).unwrap();
            assert!(painter.last_gpu_work().unwrap().draw_calls > 0);
            child.poll();
        }
        assert!(embedded.is_current());
        assert_eq!(registry.revision(), registry_revision);
        assert_eq!(embedded.host_texture().instance_identity(), identity);
    }
    assert!(
        !embedded
            .update_frame(child.sample().unwrap(), [64.0; 2])
            .unwrap()
    );
    let pixels = read_pixels(&gpu, &target);
    assert!(
        pixel(&pixels, 16, 16)[0] > 100,
        "child must really be sampled"
    );
    assert!(
        pixel(&pixels, 28, 28)[2] > 230,
        "later sibling covers embedded visual"
    );
    assert!(
        pixel(&pixels, 3, 3)[0] < 5,
        "ancestor clip excludes outside pixels"
    );
    assert!(
        pixel(&pixels, 9, 16)[0] < 5 && pixel(&pixels, 11, 16)[0] > 100,
        "parent transform moves the clip with the child"
    );
    assert!(
        (170..210).contains(&pixel(&pixels, 16, 16)[0]),
        "parent opacity applies without repainting the child: {:?}",
        pixel(&pixels, 16, 16)
    );
}

#[test]
fn registry_clone_keeps_display_storage_safe_after_node_drop_and_child_updates() {
    let gpu = test_gpu::context();
    let mut child = surface();
    let red = child_scene([1.0, 0.0, 0.0, 1.0]);
    let green = child_scene([0.0, 1.0, 0.0, 1.0]);
    let embedded = EmbeddedSurfaceNode::from_frame(render(&mut child, &red, 0), [64.0; 2]).unwrap();
    let registry = HostTextureRegistry::new();
    embedded.register_host_texture(&registry, "child", HostTextureAlphaMode::Premultiplied);
    drop(embedded);
    for revision in 1..12 {
        drop(render(&mut child, &green, revision));
    }
    let target = target(&gpu);
    let mut painter = SceneWgpuPainter::new(&gpu, GpuTextureFormat::RGBA8_UNORM);
    let mut frame = gpu.begin_frame("registry still displays old revision");
    painter
        .paint(
            &parent_scene(0.0, 1.0),
            &mut frame,
            &target.render_target().unwrap(),
            viewport(),
            Some(&registry),
            None,
        )
        .unwrap();
    frame.submit();
    let pixels = read_pixels(&gpu, &target);
    assert!(
        pixel(&pixels, 16, 16)[0] > 230 && pixel(&pixels, 16, 16)[1] < 10,
        "registry clone must keep red storage pinned"
    );
}

#[test]
fn child_replacement_retains_identity_and_mapping_without_parent_tree_mutation() {
    let mut child = surface();
    let mut embedded = EmbeddedSurfaceNode::from_frame(
        render(&mut child, &child_scene([1.0, 0.0, 0.0, 1.0]), 0),
        [64.0; 2],
    )
    .unwrap();
    let mapping = EmbeddedInputMapping {
        document: DocumentId::new(7).unwrap(),
        node: StableNodeId::new(12).unwrap(),
        revision: 8,
        clip_revision: 9,
        visible: true,
    };
    embedded.set_input_mapping(EndpointGeneration(42), Some(mapping));
    let identity = embedded.host_texture().instance_identity();
    let next = render(&mut child, &child_scene([0.0, 1.0, 0.0, 1.0]), 1);
    assert!(embedded.update_frame(next, [64.0; 2]).unwrap());
    assert_eq!(embedded.host_texture().instance_identity(), identity);
    assert_eq!(embedded.metadata().input_mapping(), Some(mapping));
    assert_eq!(
        embedded.metadata().endpoint_generation(),
        EndpointGeneration(42)
    );
    assert!(embedded.retire());
    assert!(!embedded.retire());
}

fn two_pinned_revisions() -> (
    super::super::ExternalSurface,
    EmbeddedSurfaceNode,
    ExternalFrame,
) {
    let mut child = super::super::ExternalSurface::new(
        &test_gpu::context(),
        super::super::ExternalSurfaceConfig::new(EXTENT, GpuTextureFormat::RGBA8_UNORM_SRGB)
            .with_slots(2),
    )
    .unwrap();
    let node = EmbeddedSurfaceNode::from_frame(
        render(&mut child, &child_scene([1.0, 0.0, 0.0, 1.0]), 0),
        [64.0; 2],
    )
    .unwrap();
    let newest = render(&mut child, &child_scene([0.0, 1.0, 0.0, 1.0]), 1);
    (child, node, newest)
}

fn next_preparation(child: &mut super::super::ExternalSurface) -> super::super::ExternalPrepare {
    child
        .prepare(&child_scene([0.0, 0.0, 1.0, 1.0]), 2, viewport(), None)
        .unwrap()
}

#[test]
fn detach_drops_the_nodes_own_display_lease_even_without_registry_attachments() {
    let (mut child, node, _newest) = two_pinned_revisions();
    assert!(matches!(
        next_preparation(&mut child),
        super::super::ExternalPrepare::Deferred
    ));
    assert!(
        node.retire(),
        "legacy retire disables explicit frame access"
    );
    assert!(node.texture().is_err());
    assert!(
        matches!(
            next_preparation(&mut child),
            super::super::ExternalPrepare::Deferred
        ),
        "legacy retire keeps the HostTexture display lease"
    );
    assert_eq!(node.detach(), 0);
    assert!(
        matches!(
            next_preparation(&mut child),
            super::super::ExternalPrepare::Record(_)
        ),
        "consuming detach must release its own final display reference"
    );
}

#[test]
fn detach_withdraws_only_owned_attachments_and_notifies_each_registry_outside_its_lock() {
    let (_child, node, _newest) = two_pinned_revisions();
    let first = HostTextureRegistry::new();
    let second = HostTextureRegistry::new();
    node.register_host_texture(&first, "owned", HostTextureAlphaMode::Premultiplied);
    node.register_host_texture(&first.clone(), "owned", HostTextureAlphaMode::Premultiplied);
    node.register_host_texture(&first, "replaced", HostTextureAlphaMode::Premultiplied);
    node.register_host_texture(&second, "owned", HostTextureAlphaMode::Premultiplied);
    node.register_host_texture(&second, "removed", HostTextureAlphaMode::Premultiplied);
    second.remove("removed");
    let replacement = HostTexture::new(&target(&test_gpu::context()));
    first.register(
        "replaced",
        replacement.clone(),
        64,
        64,
        HostTextureAlphaMode::Opaque,
    );
    let before = [first.revision(), second.revision()];
    let wakes = Arc::new(Mutex::new(Vec::new()));
    let subscriptions: Vec<_> = [first.clone(), second.clone()]
        .into_iter()
        .enumerate()
        .map(|(index, registry)| {
            let wakes = Arc::clone(&wakes);
            let read_back = registry.clone();
            registry.subscribe(move |slot| {
                assert!(
                    read_back.get(slot).is_none(),
                    "callbacks may acquire registry locks"
                );
                wakes.lock().unwrap().push((index, slot.to_owned()));
            })
        })
        .collect();
    assert_eq!(node.detach(), 2);
    assert!(first.get("owned").is_none() && second.get("owned").is_none());
    assert_eq!(
        first.get("replaced").unwrap().texture.instance_identity(),
        replacement.instance_identity()
    );
    assert_eq!(
        [first.revision(), second.revision()],
        [before[0] + 1, before[1] + 1]
    );
    let mut actual = wakes.lock().unwrap().clone();
    actual.sort();
    assert_eq!(actual, [(0, "owned".to_owned()), (1, "owned".to_owned())]);
    drop(subscriptions);
}

#[test]
fn detach_preserves_recorded_parent_leases_until_discard_or_actual_completion() {
    let gpu = test_gpu::context();
    for submit in [false, true] {
        let (mut child, node, _newest) = two_pinned_revisions();
        let registry = HostTextureRegistry::new();
        node.register_host_texture(&registry, "child", HostTextureAlphaMode::Premultiplied);
        let destination = target(&gpu);
        let mut painter = SceneWgpuPainter::new(&gpu, GpuTextureFormat::RGBA8_UNORM);
        let mut frame = gpu.begin_frame("detach after actual parent recording");
        painter
            .paint(
                &parent_scene(0.0, 1.0),
                &mut frame,
                &destination.render_target().unwrap(),
                viewport(),
                Some(&registry),
                None,
            )
            .unwrap();
        assert!(painter.last_gpu_work().unwrap().draw_calls > 0);
        drop(painter); // Only the recorded frame, not its CPU cache, pins red.
        assert_eq!(node.detach(), 1);
        assert!(
            matches!(
                next_preparation(&mut child),
                super::super::ExternalPrepare::Deferred
            ),
            "detach must not release a recorded parent's exact sampling lease"
        );
        if submit {
            frame.submit();
            let pixels = read_pixels(&gpu, &destination);
            assert!(pixel(&pixels, 16, 16)[0] > 230 && pixel(&pixels, 16, 16)[1] < 10);
        } else {
            frame.discard();
        }
        assert!(
            matches!(
                next_preparation(&mut child),
                super::super::ExternalPrepare::Record(_)
            ),
            "discard/completion retires the final parent reference"
        );
    }
}

#[test]
fn cloned_bindings_remain_safe_after_detach_until_their_last_reference_retires() {
    let gpu = test_gpu::context();
    let (mut child, node, _newest) = two_pinned_revisions();
    let registry = HostTextureRegistry::new();
    let saved = node.register_host_texture(&registry, "child", HostTextureAlphaMode::Premultiplied);
    let registry_clone = registry.clone();
    assert_eq!(node.detach(), 1);
    assert!(
        registry_clone.get("child").is_none(),
        "registry clones share slot authority"
    );
    assert!(
        matches!(
            next_preparation(&mut child),
            super::super::ExternalPrepare::Deferred
        ),
        "the independently retained binding is still a display lease"
    );
    let independent = HostTextureRegistry::new();
    independent.register("child", saved.texture.clone(), 64, 64, saved.alpha_mode);
    let destination = target(&gpu);
    let mut painter = SceneWgpuPainter::new(&gpu, GpuTextureFormat::RGBA8_UNORM);
    let mut frame = gpu.begin_frame("detached binding clone still displays red");
    painter
        .paint(
            &parent_scene(0.0, 1.0),
            &mut frame,
            &destination.render_target().unwrap(),
            viewport(),
            Some(&independent),
            None,
        )
        .unwrap();
    frame.submit();
    let pixels = read_pixels(&gpu, &destination);
    assert!(pixel(&pixels, 16, 16)[0] > 230 && pixel(&pixels, 16, 16)[1] < 10);
    drop(painter);
    drop(independent.remove("child"));
    drop(saved);
    assert!(matches!(
        next_preparation(&mut child),
        super::super::ExternalPrepare::Record(_)
    ));
}

struct StepBarrier {
    parties: usize,
    state: std::sync::Mutex<(usize, usize)>,
    cv: std::sync::Condvar,
}

impl StepBarrier {
    fn new(parties: usize) -> Self {
        Self {
            parties,
            state: std::sync::Mutex::new((0, 0)),
            cv: std::sync::Condvar::new(),
        }
    }

    fn wait(&self) {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let generation = state.1;
        state.0 += 1;
        if state.0 == self.parties {
            state.0 = 0;
            state.1 += 1;
            self.cv.notify_all();
            return;
        }
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while state.1 == generation {
            let remaining = deadline.saturating_duration_since(std::time::Instant::now());
            assert!(
                !remaining.is_zero(),
                "timed out waiting at concurrent_retained_refresh barrier"
            );
            let (next, _) = self
                .cv
                .wait_timeout(state, remaining)
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            state = next;
        }
    }
}

#[test]
fn concurrent_retained_refresh_never_overwrites_a_replacement_owner() {
    let (_child, node, _newest) = two_pinned_revisions();
    let registry = HostTextureRegistry::new();
    let old = node.host_texture().clone();
    let replacement = HostTexture::new(&target(&test_gpu::context()));
    let barriers = Arc::new(StepBarrier::new(3));
    let mut ownership_preserved = true;
    std::thread::scope(|scope| {
        let (refresh, refresh_old, refresh_barriers) =
            (registry.clone(), old.clone(), Arc::clone(&barriers));
        scope.spawn(move || {
            for round in 0..128 {
                refresh_barriers.wait();
                refresh.register_if_instance(
                    "child",
                    refresh_old.clone(),
                    round + 1,
                    64,
                    HostTextureAlphaMode::Premultiplied,
                );
                refresh_barriers.wait();
            }
        });
        let (replace, replacement_handle, replace_barriers) =
            (registry.clone(), replacement.clone(), Arc::clone(&barriers));
        scope.spawn(move || {
            for _ in 0..128 {
                replace_barriers.wait();
                replace.register(
                    "child",
                    replacement_handle.clone(),
                    64,
                    64,
                    HostTextureAlphaMode::Opaque,
                );
                replace_barriers.wait();
            }
        });
        for _ in 0..128 {
            registry.register(
                "child",
                old.clone(),
                64,
                64,
                HostTextureAlphaMode::Premultiplied,
            );
            barriers.wait();
            barriers.wait();
            ownership_preserved &= registry.get("child").unwrap().texture.instance_identity()
                == replacement.instance_identity();
        }
    });
    assert!(
        ownership_preserved,
        "the refresh must either precede replacement or reject the new owner"
    );
}

#[test]
fn gamma_or_untyped_srgb_output_requires_explicit_conversion() {
    let gpu = test_gpu::context();
    let mut child = super::super::ExternalSurface::new(
        &gpu,
        super::super::ExternalSurfaceConfig::new(EXTENT, GpuTextureFormat::RGBA8_UNORM),
    )
    .unwrap();
    let frame = render(&mut child, &child_scene([1.0, 0.0, 0.0, 0.5]), 0);
    assert!(matches!(
        EmbeddedSurfaceNode::from_frame(frame, [64.0; 2]),
        Err(EmbeddedMetadataError::UnsupportedSampling)
    ));
    let mut child = surface();
    child.set_alpha_encoding(AlphaEncoding::Gamma);
    let frame = render(&mut child, &child_scene([1.0, 0.0, 0.0, 0.5]), 0);
    assert!(matches!(
        EmbeddedSurfaceNode::from_frame(frame, [64.0; 2]),
        Err(EmbeddedMetadataError::UnsupportedSampling)
    ));
}

#[test]
fn replacing_child_pixels_reaches_cached_parent_without_scene_mutation() {
    let gpu = test_gpu::context();
    let mut child = surface();
    let mut embedded = EmbeddedSurfaceNode::from_frame(
        render(&mut child, &child_scene([1.0, 0.0, 0.0, 1.0]), 0),
        [64.0; 2],
    )
    .unwrap();
    let registry = HostTextureRegistry::new();
    let parent = parent_scene(0.0, 1.0);
    let projection = parent.projection_revision();
    let target = target(&gpu);
    let mut painter = SceneWgpuPainter::new(&gpu, GpuTextureFormat::RGBA8_UNORM);
    for (revision, color, channel) in [(0, [1.0, 0.0, 0.0, 1.0], 0), (1, [0.0, 1.0, 0.0, 1.0], 1)] {
        if revision != 0 {
            let next = render(&mut child, &child_scene(color), revision);
            assert!(embedded.update_frame(next, [64.0; 2]).unwrap());
        }
        embedded.register_host_texture(&registry, "child", HostTextureAlphaMode::Premultiplied);
        let mut frame = gpu.begin_frame("stable embedded replacement");
        painter
            .paint(
                &parent,
                &mut frame,
                &target.render_target().unwrap(),
                viewport(),
                Some(&registry),
                None,
            )
            .unwrap();
        frame.submit();
        let pixels = read_pixels(&gpu, &target);
        assert!(
            pixel(&pixels, 16, 16)[channel] > 230,
            "new child content reaches retained parent: {:?}",
            pixel(&pixels, 16, 16)
        );
        assert_eq!(
            parent.projection_revision(),
            projection,
            "texture replacement never changes parent scene/layout"
        );
    }
}

#[test]
fn one_child_can_be_recorded_by_multiple_parents_and_discarded() {
    let gpu = test_gpu::context();
    let mut child = surface();
    let scene = child_scene([1.0, 0.0, 0.0, 1.0]);
    let embedded =
        EmbeddedSurfaceNode::from_frame(render(&mut child, &scene, 0), [64.0; 2]).unwrap();
    let registry = HostTextureRegistry::new();
    embedded.register_host_texture(&registry, "child", HostTextureAlphaMode::Premultiplied);
    let mut parents = [
        SceneWgpuPainter::new(&gpu, GpuTextureFormat::RGBA8_UNORM),
        SceneWgpuPainter::new(&gpu, GpuTextureFormat::RGBA8_UNORM),
    ];
    let targets = [target(&gpu), target(&gpu)];
    for (index, painter) in parents.iter_mut().enumerate() {
        let mut frame = gpu.begin_frame("embedded discarded parent");
        embedded.retain_in_frame(&mut frame).unwrap();
        painter
            .paint(
                &parent_scene(0.0, 1.0),
                &mut frame,
                &targets[index].render_target().unwrap(),
                viewport(),
                Some(&registry),
                None,
            )
            .unwrap();
        frame.discard();
        let mut frame = gpu.begin_frame("embedded resubmitted parent");
        painter
            .paint(
                &parent_scene(0.0, 1.0),
                &mut frame,
                &targets[index].render_target().unwrap(),
                viewport(),
                Some(&registry),
                None,
            )
            .unwrap();
        let submission = frame.submit();
        embedded.bind_to_submission(&gpu, &submission).unwrap();
    }
    drop(registry.remove("child"));
    drop(embedded);
    for texture in targets {
        let pixels = read_pixels(&gpu, &texture);
        assert!(
            pixel(&pixels, 16, 16)[0] > 230,
            "both parents must retain the sampled child"
        );
    }
    assert!(matches!(
        child.render(&scene, 0, viewport(), None, None).unwrap(),
        super::super::ExternalRenderOutcome::Reused { .. }
    ));
}

#[test]
fn replacing_a_child_before_parent_submit_keeps_the_recorded_revision_pinned() {
    let gpu = test_gpu::context();
    let mut child = surface();
    let mut embedded = EmbeddedSurfaceNode::from_frame(
        render(&mut child, &child_scene([1.0, 0.0, 0.0, 1.0]), 0),
        [64.0; 2],
    )
    .unwrap();
    let registry = HostTextureRegistry::new();
    embedded.register_host_texture(&registry, "child", HostTextureAlphaMode::Premultiplied);
    let target = target(&gpu);
    let mut painter = SceneWgpuPainter::new(&gpu, GpuTextureFormat::RGBA8_UNORM);
    let id = crate::RenderTargetId(999);
    let mut pending = gpu.begin_frame("recorded old child; not submitted yet");
    painter
        .paint_target(
            id,
            &parent_scene(0.0, 1.0),
            &mut pending,
            &target.render_target().unwrap(),
            viewport(),
            Some(&registry),
            None,
        )
        .unwrap();
    // Drop the painter cache's display reference as well. Only the pending
    // frame may now keep the exact red storage alive after replacement.
    painter.remove_target(id);
    let green = child_scene([0.0, 1.0, 0.0, 1.0]);
    let next = render(&mut child, &green, 1);
    embedded.update_frame(next, [64.0; 2]).unwrap();
    embedded.register_host_texture(&registry, "child", HostTextureAlphaMode::Premultiplied);
    drop(render(&mut child, &green, 2));
    // Red is held by the unsubmitted parent, green by the display binding,
    // and the third slot is the latest completed version. Backpressure must
    // defer subsequent revisions, never overwrite either sampled resource.
    for revision in 3..12 {
        assert_eq!(
            child
                .render(&green, revision, viewport(), None, None)
                .unwrap(),
            super::super::ExternalRenderOutcome::Deferred
        );
        assert_eq!(child.last_work().ui_renders, 0);
    }
    pending.submit();
    let pixels = read_pixels(&gpu, &target);
    assert!(
        pixel(&pixels, 16, 16)[0] > 230 && pixel(&pixels, 16, 16)[1] < 10,
        "a pending parent must sample its recorded red revision, not recycled green storage: {:?}",
        pixel(&pixels, 16, 16)
    );
}

#[test]
fn a_delayed_child_revision_cannot_roll_back_the_persistent_binding() {
    let mut child = surface();
    let red = child_scene([1.0, 0.0, 0.0, 1.0]);
    let older = render(&mut child, &red, 0);
    let mut embedded = EmbeddedSurfaceNode::from_frame(older.clone(), [64.0; 2]).unwrap();
    let registry = HostTextureRegistry::new();
    embedded.register_host_texture(&registry, "child", HostTextureAlphaMode::Premultiplied);
    let green = child_scene([0.0, 1.0, 0.0, 1.0]);
    let newer = render(&mut child, &green, 1);
    let expected_revision = newer.content_revision();
    let expected_texture = newer.texture().unwrap().clone();
    assert!(embedded.update_frame(newer, [64.0; 2]).unwrap());
    let registry_revision = registry.revision();
    let binding_generation = registry.get("child").unwrap().texture.generation();
    assert!(
        older.is_current(),
        "the delayed lease remains a valid old revision"
    );
    assert_eq!(
        embedded.update_frame(older, [64.0; 2]),
        Err(EmbeddedMetadataError::StaleFrame)
    );
    assert_eq!(embedded.content_revision(), expected_revision);
    assert!(embedded.texture().unwrap().ptr_eq(&expected_texture));
    assert_eq!(registry.revision(), registry_revision);
    assert_eq!(
        registry.get("child").unwrap().texture.generation(),
        binding_generation
    );
}

#[test]
fn replacing_with_another_child_does_not_alias_equal_revision_numbers() {
    fn first_revision(
        surface: &mut super::super::ExternalSurface,
        scene: &UiScene,
    ) -> ExternalFrame {
        // Reserve an explicit host frame so transient global frame-slot
        // contention cannot consume a different number of prepare revisions
        // in the two independent producers this test compares.
        let mut recording = surface.gpu().begin_frame("equal-revision child fixture");
        let super::super::ExternalPrepare::Record(plan) =
            surface.prepare(scene, 0, viewport(), None).unwrap()
        else {
            panic!("new child has a free target");
        };
        surface
            .record(&plan, &mut recording, None, scene, viewport(), None, None)
            .unwrap();
        surface.bind_submission(plan, &recording.submit()).unwrap();
        render(surface, scene, 0)
    }
    let mut first = surface();
    let mut second = surface();
    let first = first_revision(&mut first, &child_scene([1.0, 0.0, 0.0, 1.0]));
    let second = first_revision(&mut second, &child_scene([0.0, 1.0, 0.0, 1.0]));
    assert_eq!(first.content_revision(), second.content_revision());
    assert_eq!(first.resource_generation(), second.resource_generation());
    let second_texture = second.texture().unwrap().clone();
    let mut embedded = EmbeddedSurfaceNode::from_frame(first, [64.0; 2]).unwrap();
    assert!(
        embedded.update_frame(second, [64.0; 2]).unwrap(),
        "revision numbers are local to a producer; a different child must replace the resource"
    );
    assert!(embedded.texture().unwrap().ptr_eq(&second_texture));
}
