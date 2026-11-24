//! Actual GPU coverage of the presenter adapter around the shared kernel.
use super::*;
use crate::{HostTextureAlphaMode, NanaTextShaper, test_gpu};
use nana_ui_core::LengthSpec;
use nana_ui_runtime::{DocumentId, GpuTextureView, LayoutViewport, Stack};
use nana_ui_scene::RuntimeDocument;
use std::collections::HashMap;

fn geometry() -> WindowGeometry {
    WindowGeometry {
        physical_position: None,
        physical_size: (64, 48),
        logical_position: None,
        logical_size: (64.0, 48.0),
        scale_factor: 1.0,
        maximized: false,
    }
}

fn scene(textured: bool) -> std::sync::Arc<UiScene> {
    scene_with_color(textured, [0.64, 0.16, 0.04, 0.5])
}

fn scene_with_color(textured: bool, color: [f32; 4]) -> std::sync::Arc<UiScene> {
    let id = DocumentId::new(242).unwrap();
    let mut document = RuntimeDocument::new(id);
    if textured {
        let mut view = GpuTextureView::new("feedback");
        let layout = std::sync::Arc::make_mut(&mut view.style.layout);
        layout.width = Some(LengthSpec::Px(64.0));
        layout.height = Some(LengthSpec::Px(48.0));
        document.context_mut().create_component(id, view).unwrap();
    } else {
        document
            .context_mut()
            .create_component(
                id,
                Stack::fill_column(0.0).with_layout(|layout| {
                    layout.background = Some(color);
                }),
            )
            .unwrap();
    }
    document
        .flush(
            LayoutViewport::new(64.0, 48.0),
            &mut NanaTextShaper::default(),
        )
        .unwrap();
    assert!(document.scene().primitive_count() > 0);
    document.shared_scene()
}

fn target() -> GpuTexture {
    test_gpu::context()
        .create_texture(&GpuTextureDescriptor {
            label: Some("actual presenter acceptance destination"),
            width: 64,
            height: 48,
            format: WINDOW_OUTPUT_FORMAT,
            usage: GpuTextureUsages::RENDER_TARGET | GpuTextureUsages::COPY_SRC,
        })
        .unwrap()
}

fn direct(viewport: ScenePaintViewport) -> DirectPresentation {
    DirectPresentation {
        id: RenderTargetId(242),
        profile: output_profile(),
        viewport,
        alpha: AlphaEncoding::Gamma,
        text_aa: None,
        parameters: Default::default(),
        damage: None,
    }
}

fn initial(
    state: &mut WindowOutputState,
    painters: &mut HashMap<ScenePresentationProfile, SceneWgpuPainter>,
    scene: &UiScene,
) -> WindowOutputFrame {
    let gpu = test_gpu::context();
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        let step = state.begin(
            &gpu,
            &geometry(),
            scene,
            0,
            false,
            None,
            [0.0; 4],
            Instant::now(),
        );
        if matches!(step, OutputStep::Record { .. }) {
            let mut frame = gpu.begin_frame("initial retained presenter revision");
            let (recorded, _) = state
                .encode_presentation(
                    Some(step),
                    None,
                    scene,
                    &mut frame,
                    &gpu,
                    painters,
                    None,
                    None,
                    None,
                    None,
                )
                .unwrap();
            let submission = frame.submit();
            return state
                .complete(recorded.unwrap(), &submission, Instant::now())
                .unwrap();
        }
        assert!(Instant::now() < deadline);
        gpu.poll();
        std::thread::sleep(Duration::from_millis(1));
    }
}

#[test]
fn suspension_clears_only_consumer_then_resumes_same_resources_and_releases() {
    let gpu = test_gpu::context();
    let config = WindowOutputConfig::default().with_extent(WindowOutputExtent::MatchWindow);
    let mut state = WindowOutputState::new(WindowId(242));
    let mut painters = HashMap::new();
    let content = scene_with_color(false, [0.5, 0.2, 0.1, 1.0]);
    state.set_config(Some(config));
    let first = initial(&mut state, &mut painters, &content);
    let generation = first.resource_generation();
    let reference = crate::output::driver_tests::read_pixels(first.texture());
    drop(first);
    let destination = target();
    let viewport = config.viewport(&geometry(), [64, 48], [0.; 4]).unwrap();
    for _ in 0..12 {
        state.set_config(Some(config.with_suspended(true)));
        assert!(matches!(
            state.frame_demand(false),
            crate::FrameDemand::At(_)
        ));
        let wake = state.frame_demand(false);
        // A freshly computed `Instant::now()` would move this deadline on
        // every schedule pass and starve a hidden static output forever.
        assert_eq!(state.frame_demand(false), wake);
        assert_eq!(wake, crate::FrameDemand::At(state.suspend_at));
        assert!(!state.can_share_window(&gpu, &geometry(), output_profile(), viewport, false));
        let step = state.begin(
            &gpu,
            &geometry(),
            &content,
            10,
            true,
            None,
            [1.; 4],
            Instant::now(),
        );
        assert!(matches!(step, OutputStep::Record { .. }));
        let mut frame = gpu.begin_frame("suspend consumer without blanking window");
        let (recorded, encoded) = state
            .encode_presentation(
                Some(step),
                Some(direct(viewport)),
                &content,
                &mut frame,
                &gpu,
                &mut painters,
                Some(&destination.render_target().unwrap()),
                None,
                None,
                None,
            )
            .unwrap();
        assert!(encoded.direct_ui && !encoded.shared);
        let submission = frame.submit();
        let blank = state
            .complete(recorded.unwrap(), &submission, Instant::now())
            .unwrap();
        assert_eq!(blank.resource_generation(), generation);
        assert!(
            crate::output::driver_tests::read_pixels(blank.texture())
                .iter()
                .all(|v| *v == 0)
        );
        assert_eq!(
            crate::output::driver_tests::read_pixels(&destination),
            reference
        );
        drop(blank);
        assert_eq!(state.frame_demand(false), crate::FrameDemand::OnDemand);
        assert!(matches!(
            state.begin(
                &gpu,
                &geometry(),
                &content,
                99,
                true,
                None,
                [1.; 4],
                Instant::now()
            ),
            OutputStep::Idle
        ));
        state.set_config(Some(config));
        let resumed = initial(&mut state, &mut painters, &content);
        assert_eq!(resumed.resource_generation(), generation);
        assert_eq!(
            crate::output::driver_tests::read_pixels(resumed.texture()),
            reference
        );
        assert_eq!(state.last_work().target_recreates, 0);
        drop(resumed);
    }
    state.set_config(None);
    assert!(state.surface.is_none());
    assert!(state.transfer.is_none());
    assert!(state.straight_targets.is_empty());
    assert_eq!(state.frame_demand(false), crate::FrameDemand::OnDemand);
}

#[test]
fn cyclic_retained_output_preserves_real_direct_pixels_for_shared_and_separate_routes() {
    let gpu = test_gpu::context();
    for extent in [
        WindowOutputExtent::MatchWindow,
        WindowOutputExtent::Fixed {
            width: 32,
            height: 24,
        },
    ] {
        let config = WindowOutputConfig::default().with_extent(extent);
        let mut state = WindowOutputState::new(WindowId(242));
        state.set_config(Some(config));
        let mut painters = HashMap::new();
        let original = initial(
            &mut state,
            &mut painters,
            &scene_with_color(false, [1.0; 4]),
        );
        let retained = original.lease.clone();
        let deadline = Instant::now() + Duration::from_secs(30);
        while state.surface.as_mut().unwrap().sample().is_err() {
            assert!(Instant::now() < deadline);
            gpu.poll();
            std::thread::sleep(Duration::from_millis(1));
        }
        let textures = HostTextureRegistry::new();
        // Opaque white is invariant under the ordinary HostTexture linear
        // sampling contract. The binding retains exact producer ancestry.
        let texture = crate::HostTexture::new_retained(
            retained.texture().unwrap(),
            std::sync::Arc::new(retained.clone()),
            retained.provenance().clone(),
        );
        let [width, height] = original.extent();
        textures.register(
            "feedback",
            texture,
            width,
            height,
            HostTextureAlphaMode::Opaque,
        );
        let feedback = scene(true);
        let step = state.begin(
            &gpu,
            &geometry(),
            &feedback,
            1,
            false,
            Some(&textures),
            [0.0; 4],
            Instant::now(),
        );
        assert!(matches!(step, OutputStep::Record { .. }));
        let viewport = WindowOutputConfig::default()
            .with_extent(WindowOutputExtent::MatchWindow)
            .viewport(&geometry(), [64, 48], [0.0; 4])
            .unwrap();
        assert_eq!(
            state.can_share_window(&gpu, &geometry(), output_profile(), viewport, false),
            extent == WindowOutputExtent::MatchWindow
        );
        state.take_statuses();
        let destination = target();
        let mut frame = gpu.begin_frame("retain healthy visible presentation despite output cycle");
        let (recorded, encoded) = state
            .encode_presentation(
                Some(step),
                Some(direct(viewport)),
                &feedback,
                &mut frame,
                &gpu,
                &mut painters,
                Some(&destination.render_target().unwrap()),
                Some(&textures),
                None,
                None,
            )
            .unwrap();
        assert!(recorded.is_none(), "cyclic output must never be published");
        assert!(
            encoded.direct_ui,
            "healthy direct target must still be encoded"
        );
        assert!(encoded.gpu_work.unwrap().draw_calls > 0);
        frame.submit();
        assert!(
            state
                .take_statuses()
                .iter()
                .any(|status| matches!(status, WindowOutputStatus::Failed { reason } if reason.contains("back into itself")))
        );
        assert_eq!(
            state
                .surface
                .as_mut()
                .unwrap()
                .sample()
                .unwrap()
                .content_revision(),
            original.content_revision()
        );
        let pixels = crate::output::driver_tests::read_pixels(&destination);
        let reference = crate::output::driver_tests::read_pixels(original.texture());
        let source_offset = ((original.extent()[1] / 2 * original.extent()[0]
            + original.extent()[0] / 2)
            * 4) as usize;
        let destination_offset = (24 * 64 + 32) * 4;
        assert!(
            reference[source_offset..source_offset + 4]
                .iter()
                .zip(&pixels[destination_offset..destination_offset + 4])
                .all(|(a, b)| a.abs_diff(*b) <= 2),
            "direct fallback must display the actual retained texture"
        );
    }
}

#[test]
fn straight_consumer_shares_premultiplied_window_render_then_converts_only_its_pixels() {
    let gpu = test_gpu::context();
    let config = WindowOutputConfig::default()
        .with_extent(WindowOutputExtent::MatchWindow)
        .with_alpha(WindowOutputAlpha::Straight);
    let mut state = WindowOutputState::new(WindowId(243));
    state.set_config(Some(config));
    let scene = scene(false);
    let viewport = config.viewport(&geometry(), [64, 48], [0.0; 4]).unwrap();
    let step = state.begin(
        &gpu,
        &geometry(),
        &scene,
        0,
        false,
        None,
        [0.0; 4],
        Instant::now(),
    );
    assert!(matches!(step, OutputStep::Record { .. }));
    assert!(state.can_share_window(&gpu, &geometry(), output_profile(), viewport, false));
    let destination = target();
    let mut painters = HashMap::new();
    let mut frame = gpu.begin_frame("one canonical render for premul window and straight consumer");
    let (recorded, encoded) = state
        .encode_presentation(
            Some(step),
            Some(direct(viewport)),
            &scene,
            &mut frame,
            &gpu,
            &mut painters,
            Some(&destination.render_target().unwrap()),
            None,
            None,
            None,
        )
        .unwrap();
    assert!(encoded.shared && !encoded.direct_ui);
    let submission = frame.submit();
    let output = state
        .complete(recorded.unwrap(), &submission, Instant::now())
        .unwrap();
    assert_eq!(state.last_work().ui_renders, 1);
    assert_eq!(state.last_work().submissions, 1);
    assert_eq!(state.last_work().gpu_convert_passes, 2);
    let premul = crate::output::driver_tests::read_pixels(&destination);
    let straight = crate::output::driver_tests::read_pixels(output.texture());
    let offset = (24 * 64 + 32) * 4;
    let a = premul[offset + 3];
    assert!((120..=136).contains(&a));
    assert_eq!(straight[offset + 3], a);
    for channel in 0..3 {
        let expected = (f32::from(premul[offset + channel]) * 255.0 / f32::from(a)).round() as u8;
        assert!(
            straight[offset + channel].abs_diff(expected) <= 2,
            "consumer conversion must unpremultiply the shared canonical pixels"
        );
    }
    assert!(
        straight[offset..offset + 3]
            .iter()
            .zip(&premul[offset..offset + 3])
            .all(|(straight, premul)| straight > premul)
    );
    let canonical = output.lease.texture().unwrap();
    assert!(!canonical.ptr_eq(output.texture()));
    assert_eq!(crate::output::driver_tests::read_pixels(canonical), premul);
}

#[test]
fn every_other_host_frame_in_flight_still_leaves_the_output_a_slot() {
    // A slow GPU (WARP) leaves every host frame but the one being recorded
    // incomplete. Unsubmitted plans hold their output slots the same way.
    let gpu = test_gpu::context();
    let config = WindowOutputConfig::default().with_extent(WindowOutputExtent::MatchWindow);
    let mut state = WindowOutputState::new(WindowId(242));
    let mut painters = HashMap::new();
    let content = scene(false);
    state.set_config(Some(config));
    drop(initial(&mut state, &mut painters, &content));
    let mut in_flight = Vec::new();
    for revision in 1..=gpu.policy().frame_slot_count() as u64 {
        let step = state.begin(
            &gpu,
            &geometry(),
            &content,
            revision,
            false,
            None,
            [0.0; 4],
            Instant::now(),
        );
        assert!(
            matches!(step, OutputStep::Record { .. }),
            "host frame {revision} deferred its output"
        );
        assert_eq!(state.last_work().deferred, 0);
        in_flight.push(step);
    }
}

#[test]
fn a_resend_records_an_unchanged_scene_again() {
    let gpu = test_gpu::context();
    let config = WindowOutputConfig::default().with_extent(WindowOutputExtent::MatchWindow);
    let mut state = WindowOutputState::new(WindowId(244));
    let mut painters = HashMap::new();
    let content = scene(false);
    state.set_config(Some(config));
    drop(initial(&mut state, &mut painters, &content));
    let begin = |state: &mut WindowOutputState| {
        state.begin(
            &gpu,
            &geometry(),
            &content,
            0,
            false,
            None,
            [0.0; 4],
            Instant::now(),
        )
    };
    // Once that frame completed, nothing changed: nothing is recorded or due.
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        gpu.poll();
        if matches!(begin(&mut state), OutputStep::Idle) && state.throttled_until.is_none() {
            break;
        }
        state.throttled_until = None;
        assert!(Instant::now() < deadline, "the output never settled");
        std::thread::yield_now();
    }
    assert_eq!(state.frame_demand(true), crate::FrameDemand::OnDemand);
    // Its consumer missed that frame: one is due shortly, and recorded.
    let now = Instant::now();
    state.resend(now);
    assert!(matches!(
        state.frame_demand(true),
        crate::FrameDemand::At(at) if at > now && at <= now + NATIVE_RETRY
    ));
    assert!(matches!(begin(&mut state), OutputStep::Record { .. }));
}
