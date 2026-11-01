//! Incompatible groups record through one shared painter into one frame.
//! Each group must paint what a driver of its own would, whatever the others
//! did to the shared glyph atlas in that frame, and attach, detach, resize,
//! discard and device replacement must leave the painter bounded and the
//! remaining groups untouched.
use super::*;
use crate::{
    NanaTextShaper, SceneWgpuPainter,
    output::{ExternalFramePlan, ExternalPrepare, ExternalSurface, ExternalSurfaceConfig},
};
use nana_gpu::{__framework as raw, GpuContext};
use nana_ui_runtime::{Icon, IconGlyph, LayoutViewport, List};

fn text_scene(label: &str) -> UiScene {
    let id = DocumentId::new(1).unwrap();
    let mut document = RuntimeDocument::new(id);
    document
        .context_mut()
        .create_component(id, Text::new(label))
        .unwrap();
    document
        .flush(
            LayoutViewport::new(64.0, 48.0),
            &mut NanaTextShaper::default(),
        )
        .unwrap();
    document.scene().clone()
}

/// [`text_scene`] with an icon after the label.
fn icon_scene(label: &str) -> UiScene {
    let id = DocumentId::new(1).unwrap();
    let mut document = RuntimeDocument::new(id);
    let context = document.context_mut();
    let root = context.create_component(id, List::new()).unwrap();
    let text = context.create_component(id, Text::new(label)).unwrap();
    context.append_child(root, text).unwrap();
    let icon = context
        .create_component(id, IconGlyph::new(Icon::File))
        .unwrap();
    context.append_child(root, icon).unwrap();
    document
        .flush(
            LayoutViewport::new(64.0, 48.0),
            &mut NanaTextShaper::default(),
        )
        .unwrap();
    document.scene().clone()
}

fn with(id: u64, profile: ScenePresentationProfile, extent: [u32; 2]) -> OutputConsumer {
    let mut endpoint = consumer(id);
    endpoint.requirements.profile = profile;
    endpoint.requirements.extent = extent;
    endpoint
}

fn bgra() -> ScenePresentationProfile {
    ScenePresentationProfile::sdr(GpuTextureFormat::BGRA8_UNORM)
}

fn pixels_of(driver: &mut PresentationDriver, endpoint: &OutputConsumer) -> Vec<u8> {
    read_pixels(completed(driver, endpoint.id).texture().unwrap())
}

/// The pixels of the revision `outcome` submitted, once it has completed:
/// until then [`completed`] still returns the previous one.
fn submitted_pixels(
    driver: &mut PresentationDriver,
    endpoint: &OutputConsumer,
    outcome: &ExternalRenderOutcome,
) -> Vec<u8> {
    let ExternalRenderOutcome::Submitted {
        content_revision, ..
    } = *outcome
    else {
        panic!("{:?} was not submitted: {outcome:?}", endpoint.id);
    };
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        let frame = completed(driver, endpoint.id);
        if frame.content_revision() == content_revision {
            return read_pixels(frame.texture().unwrap());
        }
        assert!(
            Instant::now() < deadline,
            "the submitted revision never completed"
        );
        driver.poll();
        std::thread::sleep(Duration::from_millis(1));
    }
}

/// What `endpoint` shows when it is the only group of a fresh driver.
fn alone(
    endpoint: OutputConsumer,
    scene: &UiScene,
    prepare: impl Fn(&mut PresentationDriver),
) -> Vec<u8> {
    let mut driver = PresentationDriver::new(&test_gpu::context());
    driver.configure(&[endpoint]).unwrap();
    prepare(&mut driver);
    render_fresh(&mut driver, scene);
    pixels_of(&mut driver, &endpoint)
}

/// One 64-texel page: the scale-2 group's glyphs cannot be placed beside the
/// scale-1 group's, so the second group to paint takes the first one's.
fn small_atlas(driver: &mut PresentationDriver) {
    driver
        .shared_painter()
        .use_small_text_atlas(64, 64 * 64 + 8);
}

#[test]
fn a_group_that_evicts_another_groups_glyphs_in_the_same_frame_leaves_its_pixels_intact() {
    let scene = text_scene("abcdefghij");
    let small = with(1, bgra(), [64, 48]);
    let large = with(2, bgra().with_force_float_working(), [128, 96]);
    let expected = [
        alone(small, &scene, small_atlas),
        alone(large, &scene, small_atlas),
    ];

    let mut driver = PresentationDriver::new(&test_gpu::context());
    driver.configure(&[small, large]).unwrap();
    small_atlas(&mut driver);
    let before = driver.text_glyph_counters().unwrap();
    render_fresh(&mut driver, &scene);
    assert_eq!(driver.work().output_submissions, Some(1));
    let after = driver.text_glyph_counters().unwrap();
    assert!(
        after.glyph_atlas_evict + after.atlas_relocations
            > before.glyph_atlas_evict + before.atlas_relocations,
        "the second group takes rectangles the first group's pass samples"
    );
    assert!(
        pixels_of(&mut driver, &small) == expected[0],
        "the first group samples the glyphs it placed, not the second group's"
    );
    assert!(pixels_of(&mut driver, &large) == expected[1]);
}

#[test]
fn per_group_parameters_survive_every_later_group_of_the_same_frame() {
    let groups = [
        with(1, bgra(), [64, 48]),
        with(2, bgra().with_force_float_working(), [80, 48]),
        with(3, bgra(), [48, 48]),
    ];
    let mut driver = PresentationDriver::new(&test_gpu::context());
    driver.configure(&groups).unwrap();
    assert_eq!(driver.plan().groups.len(), groups.len());
    for (frame, label) in ["Frame one", "Second frame", "Third"]
        .into_iter()
        .enumerate()
    {
        let scene = text_scene(label);
        // Parallel GPU tests share the frame slots, and a lease retires only
        // once a poll sees its GPU completion: a group may defer. It is
        // retried until every group has submitted this revision; the ones
        // that already did report it reused.
        let deadline = Instant::now() + Duration::from_secs(30);
        let mut submitted: Vec<Option<ExternalRenderOutcome>> = vec![None; groups.len()];
        let (mut renders, mut attempts) = (0, 0);
        while submitted.iter().any(Option::is_none) {
            let outcomes = driver
                .render_scene(&scene, [64.0, 48.0], frame as u64 + 1, CLEAR, None, None)
                .unwrap();
            renders += driver.work().output_ui_renders.unwrap();
            if driver.work().output_submissions == Some(1) {
                attempts += 1;
                assert_eq!(
                    driver.work().output_ui_renders,
                    Some(
                        outcomes
                            .iter()
                            .filter(|outcome| matches!(
                                outcome,
                                ExternalRenderOutcome::Submitted { .. }
                            ))
                            .count()
                    ),
                    "{label}: every group that records shares the one submission"
                );
            }
            for (slot, outcome) in submitted.iter_mut().zip(outcomes) {
                if matches!(outcome, ExternalRenderOutcome::Submitted { .. }) {
                    assert!(slot.is_none(), "{label}: a group renders a revision once");
                    *slot = Some(outcome);
                }
            }
            assert!(
                Instant::now() < deadline,
                "{label}: a group never got a target"
            );
            driver.poll();
            std::thread::sleep(Duration::from_millis(1));
        }
        assert_eq!(renders, groups.len(), "{label}: one render per group");
        assert!(attempts >= 1);
        let outcomes: Vec<_> = submitted.into_iter().map(Option::unwrap).collect();
        for (endpoint, outcome) in groups.iter().zip(&outcomes) {
            assert!(
                submitted_pixels(&mut driver, endpoint, outcome)
                    == alone(*endpoint, &scene, |_| {}),
                "{label}: group {:?} shows its own uniforms and instances",
                endpoint.id
            );
        }
    }
}

#[test]
fn unchanged_incompatible_groups_reuse_without_rendering_submitting_or_uploading() {
    let groups = [
        with(1, bgra(), EXTENT),
        with(2, bgra().with_force_float_working(), [80, 48]),
    ];
    let mut driver = PresentationDriver::new(&test_gpu::context());
    driver.configure(&groups).unwrap();
    let scene = text_scene("Static");
    assert!(
        render(&mut driver, &scene)
            .iter()
            .all(|outcome| matches!(outcome, ExternalRenderOutcome::Submitted { .. }))
    );
    let revisions: Vec<_> = groups
        .iter()
        .map(|endpoint| completed(&mut driver, endpoint.id).content_revision())
        .collect();
    let glyphs = driver.text_glyph_counters().unwrap();
    for _ in 0..60 {
        let outcomes = render(&mut driver, &scene);
        assert!(
            outcomes
                .iter()
                .all(|outcome| matches!(outcome, ExternalRenderOutcome::Reused { .. })),
            "{outcomes:?}"
        );
        let work = driver.work();
        assert_eq!(work.output_ui_renders, Some(0));
        assert_eq!(work.output_submissions, Some(0));
        assert_eq!(work.output_target_recreates, Some(0));
        assert_eq!(work.gpu_upload_bytes.unwrap_or(0), 0);
    }
    for (endpoint, revision) in groups.iter().zip(revisions) {
        assert_eq!(
            completed(&mut driver, endpoint.id).content_revision(),
            revision
        );
    }
    assert_eq!(
        driver.text_glyph_counters().unwrap().glyph_upload_bytes,
        glyphs.glyph_upload_bytes
    );
}

#[test]
fn detaching_resizing_and_reattaching_a_group_keeps_the_shared_painter_bounded() {
    let scene = icon_scene("Groups");
    let stay = with(1, bgra(), EXTENT);
    let float = with(2, bgra().with_force_float_working(), [80, 48]);
    let mut driver = PresentationDriver::new(&test_gpu::context());
    driver.configure(&[stay, float]).unwrap();
    render_fresh(&mut driver, &scene);
    let stay_frame = completed(&mut driver, stay.id);
    let stay_pixels = read_pixels(stay_frame.texture().unwrap());
    let float_frame = completed(&mut driver, float.id);
    let float_pixels = read_pixels(float_frame.texture().unwrap());
    let (two_targets, _) = driver.shared_painter().retained_shape();
    let two_icons = driver.painter_resource_counters().unwrap();

    // Detach: the float group's leases fail, its targets and pipeline set go,
    // and the remaining group keeps its pool without rendering again.
    assert!(driver.configure(&[stay]).unwrap());
    assert!(!float_frame.is_current());
    assert!(float_frame.texture().is_err());
    assert!(driver.sample(float.id).is_err());
    let kept = completed(&mut driver, stay.id);
    assert!(
        kept.texture()
            .unwrap()
            .ptr_eq(stay_frame.texture().unwrap())
    );
    assert_eq!(kept.content_revision(), stay_frame.content_revision());
    let (one_targets, sets) = driver.shared_painter().retained_shape();
    assert!(one_targets < two_targets, "{one_targets} of {two_targets}");
    assert_eq!(sets, 0, "the detached profile's pipeline set is dropped");
    let one_icons = driver.painter_resource_counters().unwrap();
    assert!(one_icons.icon_atlases < two_icons.icon_atlases);
    assert!(one_icons.icon_atlas_bytes < two_icons.icon_atlas_bytes);

    // Reattach: the group is new, and paints what it painted before.
    assert!(driver.configure(&[stay, float]).unwrap());
    render(&mut driver, &scene);
    assert!(pixels_of(&mut driver, &float) == float_pixels);
    assert!(pixels_of(&mut driver, &stay) == stay_pixels);
    let (reattached, _) = driver.shared_painter().retained_shape();

    // Resize one group: its old lease fails, the other is reused untouched,
    // and the painter holds no more targets than before.
    let old = completed(&mut driver, float.id);
    let resized = with(2, bgra().with_force_float_working(), [96, 64]);
    assert!(driver.configure(&[stay, resized]).unwrap());
    assert!(!old.is_current());
    let outcomes = render(&mut driver, &scene);
    assert!(
        matches!(outcomes[0], ExternalRenderOutcome::Reused { .. }),
        "{outcomes:?}"
    );
    assert!(matches!(
        outcomes[1],
        ExternalRenderOutcome::Submitted { .. }
    ));
    assert_eq!(driver.work().output_ui_renders, Some(1));
    assert!(pixels_of(&mut driver, &resized) == alone(resized, &scene, |_| {}));
    assert!(pixels_of(&mut driver, &stay) == stay_pixels);
    let (after_resize, _) = driver.shared_painter().retained_shape();
    assert!(
        after_resize <= reattached,
        "{after_resize} targets after resize, {reattached} before"
    );
}

/// A target's icon atlas is created by its first glyph: groups that paint
/// only text hold no sheet, and each one gets its own once an icon appears.
#[test]
fn icon_atlases_are_allocated_by_the_first_icon_a_target_draws() {
    const SHEET_BYTES: u64 = 256 * 256 * 4;
    let first = with(1, bgra(), EXTENT);
    let second = with(2, bgra().with_force_float_working(), [80, 48]);
    let mut driver = PresentationDriver::new(&test_gpu::context());
    driver.configure(&[first, second]).unwrap();
    render_fresh(&mut driver, &text_scene("Plain"));
    let plain = driver.painter_resource_counters().unwrap();
    assert_eq!(plain.icon_atlases, 0, "{plain:?}");
    assert_eq!(plain.icon_atlas_bytes, 0, "{plain:?}");

    let scene = icon_scene("Plain");
    let outcomes = render_fresh(&mut driver, &scene);
    let icons = driver.painter_resource_counters().unwrap();
    assert!(icons.icon_atlases >= 2, "one sheet per group: {icons:?}");
    assert_eq!(icons.icon_atlas_bytes, icons.icon_atlases * SHEET_BYTES);
    for (endpoint, outcome) in [first, second].iter().zip(&outcomes) {
        assert!(
            submitted_pixels(&mut driver, endpoint, outcome) == alone(*endpoint, &scene, |_| {})
        );
    }
}

#[test]
fn a_driver_refuses_another_generation_and_a_lost_device_without_submitting() {
    let shared = test_gpu::context();
    let generation = |gpu: &GpuContext| {
        raw::adopt(
            raw::adapter(gpu).clone(),
            raw::device(gpu).clone(),
            raw::queue(gpu).clone(),
        )
    };
    // Two groups, so the generation contract is the shared painter's.
    let on = |gpu: &GpuContext| {
        [
            with(1, bgra(), EXTENT),
            with(2, bgra().with_force_float_working(), [80, 48]),
        ]
        .map(|mut endpoint| {
            endpoint.requirements.device_generation = gpu.generation();
            endpoint
        })
    };
    let scene = text_scene("Generation");
    let first = generation(&shared);
    let groups = on(&first);
    let mut driver = PresentationDriver::new(&first);
    driver.configure(&groups).unwrap();
    render_fresh(&mut driver, &scene);
    let before: Vec<_> = groups
        .iter()
        .map(|endpoint| completed(&mut driver, endpoint.id))
        .collect();
    let pixels: Vec<_> = before
        .iter()
        .map(|frame| read_pixels(frame.texture().unwrap()))
        .collect();

    let mut foreign = with(3, bgra(), [96, 64]);
    foreign.requirements.device_generation = shared.generation();
    assert!(driver.configure(&[groups[0], groups[1], foreign]).is_err());
    assert_eq!(driver.plan().groups.len(), 2, "the old plan is kept");
    for (endpoint, frame) in groups.iter().zip(&before) {
        let kept = completed(&mut driver, endpoint.id);
        assert_eq!(kept.content_revision(), frame.content_revision());
    }

    raw::mark_lost(
        &first,
        nana_gpu::GpuDeviceLost {
            reason: nana_gpu::GpuLossReason::Destroyed,
            message: "shared painter generation fixture".into(),
        },
    );
    assert!(
        driver
            .render_scene(&text_scene("Lost"), [64.0, 48.0], 9, CLEAR, None, None)
            .is_err()
    );
    let work = driver.work();
    assert_eq!(work.output_submissions.unwrap_or(0), 0);
    assert_eq!(work.output_ui_renders.unwrap_or(0), 0);
    for endpoint in &groups {
        assert!(driver.sample(endpoint.id).is_err());
    }

    // A headless host replaces the driver on the new generation.
    let second = generation(&shared);
    let replacement = on(&second);
    let mut driver = PresentationDriver::new(&second);
    driver.configure(&replacement).unwrap();
    render_fresh(&mut driver, &scene);
    for (endpoint, expected) in replacement.iter().zip(&pixels) {
        assert!(&pixels_of(&mut driver, endpoint) == expected);
    }
}

/// The 64x48 scene scaled to fill `extent`: a larger extent places glyphs
/// of its own in the shared atlas.
fn surface_viewport(extent: [u32; 2]) -> crate::ScenePaintViewport {
    crate::ScenePaintViewport {
        logical_size: [64.0, 48.0],
        physical_size: extent,
        scale_factor: extent[0] as f32 / 64.0,
        scene_origin: [0.0, 0.0],
        target_origin: [0.0, 0.0],
        clear_color: [0.0, 0.0, 0.0, 1.0],
        clear: true,
    }
}

struct Group {
    surface: ExternalSurface,
    extent: [u32; 2],
}

impl Group {
    fn new(gpu: &GpuContext, profile: ScenePresentationProfile, extent: [u32; 2], ns: u64) -> Self {
        // Two slots: with one published and leased, a prepare succeeds only
        // if the other came back.
        let config = ExternalSurfaceConfig::with_presentation(extent, profile)
            .with_slots(2)
            .with_copy_source(true);
        Self {
            surface: ExternalSurface::with_host_painter(gpu, config, ns).unwrap(),
            extent,
        }
    }

    fn prepare(&mut self, scene: &UiScene, revision: u64) -> ExternalFramePlan {
        match self
            .surface
            .prepare(scene, revision, surface_viewport(self.extent), None)
            .unwrap()
        {
            ExternalPrepare::Record(plan) => plan,
            other => panic!("expected a reserved target, got {other:?}"),
        }
    }

    fn record(
        &mut self,
        plan: &ExternalFramePlan,
        frame: &mut nana_gpu::FrameContext,
        painter: &mut SceneWgpuPainter,
        scene: &UiScene,
    ) {
        self.surface
            .record(
                plan,
                frame,
                Some(painter),
                scene,
                surface_viewport(self.extent),
                None,
                None,
            )
            .unwrap();
    }

    /// The pixels of `revision` once it has completed. Until then `sample`
    /// still returns the revision published before it, so taking the first
    /// sample compares the previous frame whenever the GPU lags the test.
    fn pixels(&mut self, revision: u64) -> Vec<u8> {
        let gpu = test_gpu::context();
        let deadline = Instant::now() + Duration::from_secs(30);
        loop {
            gpu.poll();
            self.surface.poll();
            if let Ok(frame) = self.surface.sample()
                && frame.content_revision() == revision
            {
                return read_pixels(frame.texture().unwrap());
            }
            assert!(Instant::now() < deadline, "output did not complete");
            std::thread::sleep(Duration::from_millis(1));
        }
    }
}

/// Two groups recorded into one frame that is then dropped: both plans go
/// stale, both slots come back, the published versions stay, and the next
/// frame repaints both completely instead of reusing what the dropped frame
/// recorded into the shared painter.
#[test]
fn dropping_a_frame_with_two_recorded_groups_rolls_both_back() {
    let gpu = test_gpu::context();
    let profiles = [bgra(), bgra().with_force_float_working()];
    // The second group paints at scale 2, so its glyphs are its own and it
    // copies them in the frame's encoder: a dropped frame loses those copies.
    let extents = [[64, 48], [128, 96]];
    let mut painter = SceneWgpuPainter::new_with_presentation(&gpu, profiles[0]);
    let mut groups: Vec<_> = profiles
        .iter()
        .zip(extents)
        .zip(1..)
        .map(|((profile, extent), ns)| Group::new(&gpu, *profile, extent, ns << 24))
        .collect();
    let first = text_scene("Alpha");
    let second = text_scene("Omega wide");

    let mut frame = gpu.begin_frame("both groups");
    let plans: Vec<_> = groups
        .iter_mut()
        .map(|group| {
            let plan = group.prepare(&first, 0);
            (group, plan)
        })
        .map(|(group, plan)| {
            group.record(&plan, &mut frame, &mut painter, &first);
            plan
        })
        .collect();
    let revisions: Vec<_> = plans.iter().map(|plan| plan.content_revision()).collect();
    let submission = frame.submit();
    for (group, plan) in groups.iter_mut().zip(plans) {
        group.surface.bind_submission(plan, &submission).unwrap();
    }
    let published: Vec<_> = groups
        .iter_mut()
        .zip(revisions)
        .map(|(group, revision)| {
            let pixels = group.pixels(revision);
            (group.surface.sample().unwrap(), pixels)
        })
        .collect();

    let mut frame = gpu.begin_frame("dropped with both groups");
    let mut dropped = Vec::new();
    for group in &mut groups {
        let plan = group.prepare(&second, 1);
        group.record(&plan, &mut frame, &mut painter, &second);
        dropped.push(plan);
    }
    frame.discard();
    for ((group, plan), (lease, pixels)) in groups.iter_mut().zip(&dropped).zip(&published) {
        assert!(matches!(
            group.surface.plan_texture(plan),
            Err(ExternalSurfaceError::StalePlan)
        ));
        assert!(lease.is_current(), "the published lease survives the drop");
        let sample = group.surface.sample().unwrap();
        assert_eq!(sample.content_revision(), lease.content_revision());
        assert!(&read_pixels(sample.texture().unwrap()) == pixels);
    }

    // The dropped plans are still held: only the discard released their slots.
    let mut frame = gpu.begin_frame("after the drop");
    let mut plans = Vec::new();
    for group in &mut groups {
        let plan = group.prepare(&second, 2);
        group.record(&plan, &mut frame, &mut painter, &second);
        plans.push(plan);
    }
    drop(dropped);
    let revisions: Vec<_> = plans.iter().map(|plan| plan.content_revision()).collect();
    let submission = frame.submit();
    for (group, plan) in groups.iter_mut().zip(plans) {
        group.surface.bind_submission(plan, &submission).unwrap();
    }
    for (((profile, extent), group), revision) in
        profiles.iter().zip(extents).zip(&mut groups).zip(revisions)
    {
        let mut own = SceneWgpuPainter::new_with_presentation(&gpu, *profile);
        let mut reference = Group::new(&gpu, *profile, extent, 9 << 24);
        let plan = reference.prepare(&second, 0);
        let reference_revision = plan.content_revision();
        let mut frame = gpu.begin_frame("reference");
        reference.record(&plan, &mut frame, &mut own, &second);
        let submission = frame.submit();
        reference
            .surface
            .bind_submission(plan, &submission)
            .unwrap();
        assert!(
            group.pixels(revision) == reference.pixels(reference_revision),
            "{profile:?}: the frame after the drop repaints the group completely"
        );
    }
}
