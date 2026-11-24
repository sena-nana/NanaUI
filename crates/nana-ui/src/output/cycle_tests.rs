use super::*;
use crate::{
    HostTextureAlphaMode, HostTextureRegistry, NanaTextShaper, ScenePaintViewport, test_gpu,
};
use nana_gpu::GpuTextureFormat;
use nana_ui_core::LengthSpec;
use nana_ui_runtime::{DocumentId, GpuTextureView, LayoutViewport, Stack};
use nana_ui_scene::{RuntimeDocument, UiScene};
use std::sync::Arc;
use std::time::{Duration, Instant};

fn surface() -> ExternalSurface {
    ExternalSurface::new(
        &test_gpu::context(),
        ExternalSurfaceConfig::new([32, 24], GpuTextureFormat::RGBA8_UNORM_SRGB)
            .with_copy_source(true),
    )
    .unwrap()
}

fn viewport() -> ScenePaintViewport {
    ScenePaintViewport {
        logical_size: [32.0, 24.0],
        physical_size: [32, 24],
        scale_factor: 1.0,
        scene_origin: [0.0; 2],
        target_origin: [0.0; 2],
        clear_color: [0.1, 0.2, 0.3, 1.0],
        clear: true,
    }
}

fn textured_scene() -> Arc<UiScene> {
    let id = DocumentId::new(1).unwrap();
    let mut document = RuntimeDocument::new(id);
    let mut view = GpuTextureView::new("source");
    let layout = Arc::make_mut(&mut view.style.layout);
    layout.width = Some(LengthSpec::Px(32.0));
    layout.height = Some(LengthSpec::Px(24.0));
    document.context_mut().create_component(id, view).unwrap();
    document
        .flush(
            LayoutViewport::new(32.0, 24.0),
            &mut NanaTextShaper::default(),
        )
        .unwrap();
    assert!(
        !document
            .scene()
            .frame_plan()
            .unwrap()
            .custom_nodes
            .is_empty()
    );
    document.shared_scene()
}

fn colored_scene(color: [f32; 4]) -> Arc<UiScene> {
    let id = DocumentId::new(1).unwrap();
    let mut document = RuntimeDocument::new(id);
    document
        .context_mut()
        .create_component(
            id,
            Stack::fill_column(0.0).with_layout(|layout| {
                layout.paint_colors.background = Some(nana_ui_core::PaintColor::srgb(color));
            }),
        )
        .unwrap();
    document
        .flush(
            LayoutViewport::new(32.0, 24.0),
            &mut NanaTextShaper::default(),
        )
        .unwrap();
    document.shared_scene()
}

fn complete(
    surface: &mut ExternalSurface,
    scene: &UiScene,
    registry: Option<&HostTextureRegistry>,
    revision: u64,
) -> ExternalFrame {
    let deadline = Instant::now() + Duration::from_secs(30);
    let mut wanted = None;
    loop {
        if wanted.is_none() {
            match surface
                .render(scene, revision, viewport(), registry, None)
                .unwrap()
            {
                ExternalRenderOutcome::Submitted {
                    content_revision, ..
                }
                | ExternalRenderOutcome::Reused {
                    content_revision, ..
                } => wanted = Some(content_revision),
                ExternalRenderOutcome::Deferred => {}
            }
        }
        if let Ok(frame) = surface.sample()
            && Some(frame.content_revision()) == wanted
        {
            return frame;
        }
        assert!(
            Instant::now() < deadline,
            "composition output did not complete"
        );
        surface.poll();
        std::thread::sleep(Duration::from_millis(1));
    }
}

fn register(
    frame: ExternalFrame,
    registry: &HostTextureRegistry,
    slot: &str,
) -> EmbeddedSurfaceNode {
    let node = EmbeddedSurfaceNode::from_frame(frame, [32.0, 24.0]).unwrap();
    node.register_host_texture(registry, slot, HostTextureAlphaMode::Premultiplied);
    node
}

fn cycle_error(
    surface: &mut ExternalSurface,
    scene: &UiScene,
    registry: &HostTextureRegistry,
    revision: u64,
) -> ExternalSurfaceError {
    // Validate recording with a host-owned frame; temporary contention in
    // the process-shared GPU must not bypass this test through Deferred.
    let mut recording = surface.gpu().begin_frame("cyclic output recording fixture");
    let ExternalPrepare::Record(plan) = surface
        .prepare(scene, revision, viewport(), Some(registry))
        .unwrap()
    else {
        panic!("cyclic fixture has a free retained target");
    };
    surface
        .record(
            &plan,
            &mut recording,
            None,
            scene,
            viewport(),
            Some(registry),
            None,
        )
        .unwrap_err()
}

#[test]
fn a_completed_revision_cannot_be_sampled_back_into_its_own_producer() {
    let mut output = surface();
    let initial = complete(&mut output, &UiScene::new(), None, 0);
    assert_eq!(initial.provenance().source(), output.id());
    let old_revision = initial.content_revision();
    let registry = HostTextureRegistry::new();
    let _node = register(initial, &registry, "source");
    let error = cycle_error(&mut output, &textured_scene(), &registry, 1);
    assert_eq!(
        error,
        ExternalSurfaceError::CompositionCycle {
            surface: output.id()
        }
    );
    assert_eq!(output.last_work().ui_renders, 0);
    assert_eq!(output.last_work().submissions, 0);
    assert_eq!(output.published_revision(), Some(old_revision));
}

#[test]
fn revision_ancestry_rejects_a_to_b_to_a_feedback_before_gpu_recording() {
    let scene = textured_scene();
    let mut a = surface();
    let mut b = surface();
    let frame_a = complete(&mut a, &UiScene::new(), None, 0);
    let registry_b = HostTextureRegistry::new();
    let _a_node = register(frame_a, &registry_b, "source");
    let frame_b = complete(&mut b, &scene, Some(&registry_b), 1);
    assert!(frame_b.provenance().ancestry().contains(&a.id()));
    assert!(frame_b.provenance().ancestry().contains(&b.id()));
    let registry_a = HostTextureRegistry::new();
    let _b_node = register(frame_b, &registry_a, "source");
    assert!(matches!(cycle_error(&mut a, &scene, &registry_a, 2),
        ExternalSurfaceError::CompositionCycle { surface } if surface == a.id()));
    assert_eq!(a.last_work().ui_renders, 0);
    assert_eq!(a.last_work().submissions, 0);
}

#[test]
fn multiple_parents_share_a_child_and_unused_registry_slots_do_not_create_false_cycles() {
    let scene = textured_scene();
    let mut child = surface();
    let child_frame = complete(&mut child, &UiScene::new(), None, 0);
    let registry = HostTextureRegistry::new();
    let _child_node = register(child_frame.clone(), &registry, "source");
    let mut first_parent = surface();
    let mut second_parent = surface();
    let own_frame = complete(&mut first_parent, &UiScene::new(), None, 0);
    let _unused_self = register(own_frame, &registry, "unused");
    let first = complete(&mut first_parent, &scene, Some(&registry), 1);
    let second = complete(&mut second_parent, &scene, Some(&registry), 1);
    assert_eq!(first.provenance().ancestry().len(), 2);
    assert_eq!(second.provenance().ancestry().len(), 2);
    assert!(first.provenance().ancestry().contains(&child.id()));
    assert!(second.provenance().ancestry().contains(&child.id()));
    assert!(!second.provenance().ancestry().contains(&first_parent.id()));
}

#[test]
fn discarding_a_composition_does_not_publish_its_dependency_closure() {
    let mut child = surface();
    let child_frame = complete(&mut child, &UiScene::new(), None, 0);
    let registry = HostTextureRegistry::new();
    let _node = register(child_frame, &registry, "source");
    let scene = textured_scene();
    let mut parent = surface();
    let old = complete(&mut parent, &UiScene::new(), None, 0);
    let ExternalPrepare::Record(plan) = parent
        .prepare(&scene, 1, viewport(), Some(&registry))
        .unwrap()
    else {
        panic!("available target");
    };
    let mut recording = test_gpu::context().begin_frame("discarded composition");
    parent
        .record(
            &plan,
            &mut recording,
            None,
            &scene,
            viewport(),
            Some(&registry),
            None,
        )
        .unwrap();
    recording.discard();
    drop(plan);
    let frame = parent.sample().unwrap();
    assert_eq!(frame.content_revision(), old.content_revision());
    assert_eq!(frame.provenance().ancestry(), &[parent.id()]);
}

#[test]
fn resource_snapshot_keeps_the_checked_ancestry_and_child_identity_is_not_just_revision() {
    fn first_revision(surface: &mut ExternalSurface, scene: &UiScene) -> ExternalFrame {
        let mut recording = surface.gpu().begin_frame("equal-revision ancestry fixture");
        let ExternalPrepare::Record(plan) = surface.prepare(scene, 0, viewport(), None).unwrap()
        else {
            panic!("new child has a free target");
        };
        surface
            .record(&plan, &mut recording, None, scene, viewport(), None, None)
            .unwrap();
        surface.bind_submission(plan, &recording.submit()).unwrap();
        complete(surface, scene, None, 0)
    }
    let mut a = surface();
    let mut b = surface();
    let first = first_revision(&mut a, &UiScene::new());
    let second = first_revision(&mut b, &UiScene::new());
    assert_eq!(first.content_revision(), second.content_revision());
    assert_eq!(first.resource_generation(), second.resource_generation());
    let registry = HostTextureRegistry::new();
    let mut node = register(first, &registry, "source");
    let (frozen, ancestry) = registry.snapshot_output_bindings(["source"].into_iter());
    assert_eq!(ancestry[0].source(), a.id());
    assert!(node.update_frame(second, [32.0, 24.0]).unwrap());
    node.register_host_texture(&registry, "source", HostTextureAlphaMode::Premultiplied);
    let (_, frozen_ancestry) = frozen.snapshot_output_bindings(["source"].into_iter());
    let (_, current_ancestry) = registry.snapshot_output_bindings(["source"].into_iter());
    assert_eq!(frozen_ancestry[0].source(), a.id());
    assert_eq!(current_ancestry[0].source(), b.id());
    assert_eq!(node.metadata().source(), Some(b.id()));
}

#[test]
fn prepared_scene_viewport_and_registry_cannot_be_replaced_at_record_time() {
    let mut output = surface();
    let prepared_scene = UiScene::new();
    let other_scene = UiScene::new();
    let ExternalPrepare::Record(plan) = output
        .prepare(&prepared_scene, 0, viewport(), None)
        .unwrap()
    else {
        panic!("available target");
    };
    let mut frame = test_gpu::context().begin_frame("stale output inputs");
    assert_eq!(
        output.record(
            &plan,
            &mut frame,
            None,
            &other_scene,
            viewport(),
            None,
            None
        ),
        Err(ExternalSurfaceError::StalePlan)
    );
    let mut changed_viewport = viewport();
    changed_viewport.scene_origin = [1.0, 0.0];
    assert_eq!(
        output.record(
            &plan,
            &mut frame,
            None,
            &prepared_scene,
            changed_viewport,
            None,
            None
        ),
        Err(ExternalSurfaceError::StalePlan)
    );
    let registry = HostTextureRegistry::new();
    assert_eq!(
        output.record(
            &plan,
            &mut frame,
            None,
            &prepared_scene,
            viewport(),
            Some(&registry),
            None
        ),
        Err(ExternalSurfaceError::StalePlan)
    );
    assert_eq!(output.last_work().ui_renders, 0);
    frame.discard();
}

#[test]
fn child_replacement_wakes_a_static_parent_and_updates_real_pixels_without_manual_registration() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    let mut child = surface();
    let red = complete(&mut child, &colored_scene([1.0, 0.0, 0.0, 1.0]), None, 0);
    let registry = HostTextureRegistry::new();
    let mut node = register(red, &registry, "source");
    let notifications = Arc::new(AtomicUsize::new(0));
    let observed = Arc::clone(&notifications);
    let _subscription = registry.subscribe(move |_| {
        observed.fetch_add(1, Ordering::Relaxed);
    });
    let scene = textured_scene();
    let mut parent = surface();
    let before = complete(&mut parent, &scene, Some(&registry), 0);
    let before_pixels = super::driver_tests::read_pixels(before.texture().unwrap());
    let revision = registry.revision();
    let green = complete(&mut child, &colored_scene([0.0, 1.0, 0.0, 1.0]), None, 1);
    assert!(node.update_frame(green, [32.0, 24.0]).unwrap());
    assert!(registry.revision() > revision);
    assert_eq!(notifications.load(Ordering::Relaxed), 1);
    let after = complete(&mut parent, &scene, Some(&registry), 0);
    let after_pixels = super::driver_tests::read_pixels(after.texture().unwrap());
    assert!(after.content_revision() > before.content_revision());
    assert_ne!(before_pixels, after_pixels);
    assert!(
        before_pixels
            .as_chunks::<4>()
            .0
            .iter()
            .any(|pixel| pixel[0] > 240 && pixel[1] < 10)
    );
    assert!(
        after_pixels
            .as_chunks::<4>()
            .0
            .iter()
            .any(|pixel| pixel[1] > 240 && pixel[0] < 10)
    );
    registry.remove("source");
    let revision = registry.revision();
    let third = complete(&mut child, &colored_scene([0.0, 0.0, 1.0, 1.0]), None, 2);
    node.update_frame(third, [32.0, 24.0]).unwrap();
    assert_eq!(
        registry.revision(),
        revision,
        "removed slots must not be resurrected"
    );
    assert!(registry.get("source").is_none());
}
